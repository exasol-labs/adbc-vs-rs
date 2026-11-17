//! The scalar functions Exasol delegates, resolved into dialect-neutral
//! operations.
//!
//! Exasol overloads `ADD`, `SUB`, and `MULT` by operand type: a number added
//! to a date counts days, a date minus a date counts the days between them, and
//! a timestamp minus a timestamp is an interval. The request carries no type
//! for a nested node, so this module derives a [`ValueClass`] for every
//! expression and resolves each operator into the one operation it means, so a
//! dialect renders only the source syntax of an operation already decided.

use serde_json::Value;

use super::{Expression, InvolvedTable, Literal};
use crate::exa_type::ExasolType;

/// The class of value an expression yields: what decides which operation an
/// overloaded operator means and how Exasol converts an operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueClass {
    /// An exact number of `precision` digits, `scale` of them fractional, as
    /// Exasol types it.
    Exact {
        precision: u32,
        scale: u32,
    },
    Double,
    Character,
    Boolean,
    Date,
    Timestamp,
    /// An `INTERVAL DAY TO SECOND`, which only a timestamp difference yields.
    Interval,
}

/// The widest precision an Exasol `DECIMAL` holds, which caps every derived
/// precision.
const MAX_PRECISION: u32 = 36;

impl ValueClass {
    /// A whole number of up to `precision` digits.
    pub const fn integer(precision: u32) -> Self {
        Self::Exact {
            precision,
            scale: 0,
        }
    }

    pub fn of_type(exasol_type: ExasolType) -> Self {
        match exasol_type {
            ExasolType::Decimal { precision, scale } => Self::Exact { precision, scale },
            ExasolType::Double => Self::Double,
            ExasolType::Varchar { .. } | ExasolType::Char { .. } => Self::Character,
            ExasolType::Boolean => Self::Boolean,
            ExasolType::Date => Self::Date,
            ExasolType::Timestamp { .. } => Self::Timestamp,
        }
    }

    pub fn of_literal(literal: &Literal) -> Self {
        match literal {
            // A null literal takes part in arithmetic as a number.
            Literal::Null => Self::integer(1),
            Literal::ExactNumeric(value) => {
                let digits = value.trim_start_matches(['-', '+']);
                let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
                let whole = whole.trim_start_matches('0').len() as u32;
                let scale = (fraction.len() as u32).min(MAX_PRECISION);
                Self::Exact {
                    precision: (whole + scale).clamp(1, MAX_PRECISION),
                    scale,
                }
            }
            Literal::Double(_) => Self::Double,
            Literal::String(_) => Self::Character,
            Literal::Date(_) => Self::Date,
            Literal::Timestamp(_) => Self::Timestamp,
        }
    }

    fn is_temporal(self) -> bool {
        matches!(self, Self::Date | Self::Timestamp)
    }

    /// Whether Exasol reads the value as a number in arithmetic.
    fn is_number(self) -> bool {
        matches!(
            self,
            Self::Exact { .. } | Self::Double | Self::Character | Self::Boolean
        )
    }
}

/// An argument of a scalar function, carrying its class so a dialect converts
/// it as Exasol converts it.
#[derive(Debug, Clone, PartialEq)]
pub struct Operand {
    pub expression: Expression,
    pub class: ValueClass,
}

impl Operand {
    fn of(expression: Expression) -> Self {
        let class = expression.value_class();
        Self { expression, class }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    Year,
    Month,
    Day,
}

impl DatePart {
    /// The SQL name of the part, which every target spells the same.
    pub fn name(self) -> &'static str {
        match self {
            Self::Year => "YEAR",
            Self::Month => "MONTH",
            Self::Day => "DAY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
}

impl Operator {
    /// The protocol name of the operator, as an error quotes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Add => "ADD",
            Self::Subtract => "SUB",
            Self::Multiply => "MULT",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
        }
    }
}

/// Whether an operation shifts or sums forward or backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    fn of(operator: Operator) -> Self {
        match operator {
            Operator::Subtract => Self::Backward,
            Operator::Add | Operator::Multiply => Self::Forward,
        }
    }

    pub fn sign(self) -> &'static str {
        match self {
            Self::Forward => "+",
            Self::Backward => "-",
        }
    }
}

/// A `function_scalar` node this release delegates.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarCall {
    /// `YEAR`, `MONTH`, or `DAY`. A character operand reads as a timestamp.
    DatePart {
        part: DatePart,
        operand: Operand,
    },
    Upper(Operand),
    Lower(Operand),
    Trim {
        operand: Operand,
        characters: Option<Operand>,
    },
    Length(Operand),
    /// `SUBSTR` with Exasol's position rule: a position rounds to a whole
    /// number, 0 reads as 1, a negative one counts from the end, and one left
    /// of the first character or a length below 1 yields null.
    Substr {
        operand: Operand,
        start: Operand,
        length: Option<Operand>,
    },
    Arithmetic(Arithmetic),
}

/// `ADD`, `SUB`, or `MULT`, resolved by the classes of its operands.
#[derive(Debug, Clone, PartialEq)]
pub enum Arithmetic {
    /// Two exact numbers, a boolean reading as 1 or 0.
    Exact {
        operator: Operator,
        left: Operand,
        right: Operand,
    },
    /// Either operand a double or text read as a number: computed in double
    /// precision.
    Double {
        operator: Operator,
        left: Operand,
        right: Operand,
    },
    /// A date or timestamp shifted by a number of days, which Exasol rounds
    /// half away from zero to a whole number.
    ShiftDays {
        temporal: Operand,
        days: Operand,
        direction: Direction,
    },
    /// `DATE` minus `DATE`: the whole days between them.
    DaysBetween {
        left: Operand,
        right: Operand,
    },
    /// A timestamp minus a timestamp, a date reading as midnight: an interval.
    TimestampDifference {
        left: Operand,
        right: Operand,
    },
    /// A date or timestamp shifted by an interval. A date stays a date.
    ShiftInterval {
        temporal: Operand,
        interval: Operand,
        direction: Direction,
    },
    IntervalSum {
        left: Operand,
        right: Operand,
        direction: Direction,
    },
    IntervalScale {
        interval: Operand,
        factor: Operand,
    },
}

impl ScalarCall {
    pub fn value_class(&self) -> ValueClass {
        match self {
            // Exasol declares a date part `DECIMAL(4,0)` and a length `DECIMAL(9,0)`.
            Self::DatePart { .. } => ValueClass::integer(4),
            Self::Length(_) => ValueClass::integer(9),
            Self::Upper(_) | Self::Lower(_) | Self::Trim { .. } | Self::Substr { .. } => {
                ValueClass::Character
            }
            Self::Arithmetic(arithmetic) => arithmetic.value_class(),
        }
    }
}

impl Arithmetic {
    pub fn value_class(&self) -> ValueClass {
        match self {
            Self::Exact {
                operator,
                left,
                right,
            } => {
                let ((p1, s1), (p2, s2)) = (exact_digits(left.class), exact_digits(right.class));
                let (precision, scale) = match operator {
                    Operator::Multiply => (p1 + p2, s1 + s2),
                    Operator::Add | Operator::Subtract => {
                        ((p1 - s1).max(p2 - s2) + s1.max(s2) + 1, s1.max(s2))
                    }
                };
                // Exasol refuses a scale beyond 36, so no delegated value carries one.
                let scale = scale.min(MAX_PRECISION);
                ValueClass::Exact {
                    precision: precision.clamp(scale, MAX_PRECISION),
                    scale,
                }
            }
            Self::Double { .. } => ValueClass::Double,
            Self::ShiftDays { temporal, .. } | Self::ShiftInterval { temporal, .. } => {
                temporal.class
            }
            Self::DaysBetween { .. } => ValueClass::integer(9),
            Self::TimestampDifference { .. }
            | Self::IntervalSum { .. }
            | Self::IntervalScale { .. } => ValueClass::Interval,
        }
    }
}

/// The precision and scale of an exact operand, a boolean reading as one
/// digit.
fn exact_digits(class: ValueClass) -> (u32, u32) {
    match class {
        ValueClass::Exact { precision, scale } => (precision, scale),
        _ => (1, 0),
    }
}

/// The scalar functions this release renders, by protocol name, which is also
/// the list an error offers for a name outside it.
const SCALAR_FUNCTIONS: &str =
    "YEAR, MONTH, DAY, UPPER, LOWER, TRIM, LENGTH, SUBSTR, ADD, SUB, and MULT";

/// Reads a `function_scalar` node. The name is matched in upper case, as an
/// aggregate's is, because Exasol has sent aggregate names in either case.
pub(super) fn read_scalar_call(
    node: &Value,
    tables: &[InvolvedTable],
) -> Result<ScalarCall, String> {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("function_scalar node {node} names no function"))?
        .to_uppercase();
    let arguments = node
        .get("arguments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|argument| super::read_expression(argument, tables).map(Operand::of))
        .collect::<Result<Vec<_>, _>>()?;

    let count = arguments.len();
    let arity = |range: std::ops::RangeInclusive<usize>| -> Result<(), String> {
        if range.contains(&count) {
            Ok(())
        } else {
            Err(format!(
                "the scalar function `{name}` carries {count} arguments, and this release renders \
                 it over {} to {}",
                range.start(),
                range.end()
            ))
        }
    };
    let mut arguments = arguments.into_iter();
    let mut next = || arguments.next().expect("arity checked");

    match name.as_str() {
        "YEAR" | "MONTH" | "DAY" => {
            arity(1..=1)?;
            let part = match name.as_str() {
                "YEAR" => DatePart::Year,
                "MONTH" => DatePart::Month,
                _ => DatePart::Day,
            };
            Ok(ScalarCall::DatePart {
                part,
                operand: next(),
            })
        }
        "UPPER" => arity(1..=1).map(|()| ScalarCall::Upper(next())),
        "LOWER" => arity(1..=1).map(|()| ScalarCall::Lower(next())),
        "LENGTH" => arity(1..=1).map(|()| ScalarCall::Length(next())),
        "TRIM" => {
            arity(1..=2)?;
            let operand = next();
            Ok(ScalarCall::Trim {
                operand,
                characters: arguments.next(),
            })
        }
        "SUBSTR" => {
            arity(2..=3)?;
            let operand = next();
            let start = next();
            Ok(ScalarCall::Substr {
                operand,
                start,
                length: arguments.next(),
            })
        }
        "ADD" | "SUB" | "MULT" => {
            arity(2..=2)?;
            let operator = match name.as_str() {
                "ADD" => Operator::Add,
                "SUB" => Operator::Subtract,
                _ => Operator::Multiply,
            };
            let left = next();
            let right = next();
            resolve_arithmetic(operator, left, right).map(ScalarCall::Arithmetic)
        }
        other => Err(format!(
            "the scalar function `{other}` has no rendering path in this release, which renders \
             {SCALAR_FUNCTIONS} only"
        )),
    }
}

/// Resolves an operator by its operands' classes into the operation Exasol
/// performs, or fails for a combination Exasol does not admit.
fn resolve_arithmetic(
    operator: Operator,
    left: Operand,
    right: Operand,
) -> Result<Arithmetic, String> {
    use ValueClass::{Date, Interval};

    let (l, r) = (left.class, right.class);
    let arithmetic = match operator {
        _ if l.is_number() && r.is_number() => {
            if [l, r]
                .iter()
                .any(|class| matches!(class, ValueClass::Double | ValueClass::Character))
            {
                Arithmetic::Double {
                    operator,
                    left,
                    right,
                }
            } else {
                Arithmetic::Exact {
                    operator,
                    left,
                    right,
                }
            }
        }
        Operator::Add | Operator::Subtract if l.is_temporal() && r.is_number() => {
            Arithmetic::ShiftDays {
                temporal: left,
                days: right,
                direction: Direction::of(operator),
            }
        }
        Operator::Add if l.is_number() && r.is_temporal() => Arithmetic::ShiftDays {
            temporal: right,
            days: left,
            direction: Direction::Forward,
        },
        Operator::Subtract if l == Date && r == Date => Arithmetic::DaysBetween { left, right },
        Operator::Subtract if l.is_temporal() && r.is_temporal() => {
            Arithmetic::TimestampDifference { left, right }
        }
        Operator::Add | Operator::Subtract if l.is_temporal() && r == Interval => {
            Arithmetic::ShiftInterval {
                temporal: left,
                interval: right,
                direction: Direction::of(operator),
            }
        }
        Operator::Add if l == Interval && r.is_temporal() => Arithmetic::ShiftInterval {
            temporal: right,
            interval: left,
            direction: Direction::Forward,
        },
        Operator::Add | Operator::Subtract if l == Interval && r == Interval => {
            Arithmetic::IntervalSum {
                left,
                right,
                direction: Direction::of(operator),
            }
        }
        Operator::Multiply if l == Interval && r.is_number() => Arithmetic::IntervalScale {
            interval: left,
            factor: right,
        },
        Operator::Multiply if l.is_number() && r == Interval => Arithmetic::IntervalScale {
            interval: right,
            factor: left,
        },
        _ => {
            return Err(format!(
                "the scalar function `{}` over a {l:?} and a {r:?} operand has no rendering path \
                 in this release",
                operator.name()
            ))
        }
    };
    Ok(arithmetic)
}
