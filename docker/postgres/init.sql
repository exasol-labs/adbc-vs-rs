-- Core-type fixture for the PostgreSQL integration tier (`postgres-it`).
-- `orders` holds the core types `postgres_type_mapping.rs` maps to the Exasol
-- type of the same kind: `integer`, `bigint`, `numeric(p,s)`,
-- `character varying(n)`, `date`, `timestamp without time zone`, and
-- `double precision`. `type_coverage` below holds the rest.
-- `amount` carries a NULL and a maximum-scale value, the same shape
-- `docker/mysql/init.sql`'s `amount` column uses, so the Arrow
-- decimal-coercion path is exercised the same way for both sources; the
-- driver returns this column as an Arrow string rather than a decimal-typed
-- array, which is the string-form numeric path `coerce.rs` treats as
-- primary.
-- `created_at` carries a distinct microsecond component per row, since
-- PostgreSQL's `timestamp` holds microsecond precision and the coercion
-- normalizes every unit the driver may return down to it.
-- Task 8.1 grows this further into the differential E2E fixture.

CREATE TABLE orders (
    id INTEGER NOT NULL,
    big_id BIGINT NOT NULL,
    amount NUMERIC(12,4) NULL,
    name VARCHAR(100) NOT NULL,
    order_date DATE NOT NULL,
    created_at TIMESTAMP NOT NULL,
    price DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO orders (id, big_id, amount, name, order_date, created_at, price) VALUES
    (1, 9000000001, 1234.5600,     'Widget', '2026-01-15', '2026-01-15 08:30:00.123456', 19.99),
    (2, 9000000002, NULL,          'Gadget', '2026-02-20', '2026-02-20 14:45:00.000001', 42.5),
    (3, 9000000003, 99999999.9999, 'Gizmo',  '2026-03-25', '2026-03-25 23:59:59.999999', 0.01);

-- Differential end-to-end fixture (task 8.1). `orders` above is left exactly
-- as task 7.2 seeded it — `postgres_it_test.rs` pins its three rows by value
-- — and every table below is new.
--
-- `items` mixes mapped columns of every kind with one column that stays
-- unmapped, so the omission test has a table that is *partly* mapped rather
-- than only the all-or-nothing case `all_unmapped` covers. Every column but
-- `labels` is mapped and is reproduced in Exasol by
-- `crates/adbc-vs-adapter/tests/common/postgres_mirror.rs` under schema
-- `PG_MIRROR`, table `items`, seeded from byte-identical literals, with the
-- values Exasol reads: `code` as `CHAR(8)`, `details`, `photo`, and `uid` as
-- PostgreSQL's text for them, and `seen_at` as its UTC wall-clock
-- `TIMESTAMP(6)`. Keep the two in sync by hand if either changes, the same
-- obligation `common/items.rs`'s header states for the MySQL fixture.
-- `labels` is the one array column, which no mapping maps.
--
-- `category` holds the exact case-only/accent-only set and row
-- multiplicities gate 2.3(e) measured returning Exasol's four groups under
-- Exasol's comparison collation: `widget` x2, `Widget` x2, `cafe` x1,
-- `cafe` (accented) x1.
-- `sort_key` holds the `B`/`a`/`C` triple gate 2.3(d) measured PostgreSQL's
-- bare `en_US.utf8` ordering disagreeing with Exasol's byte-order ordering
-- over, seeded out of sorted order so a delegated `ORDER BY`/`MIN`/`MAX`
-- cannot pass on storage order alone.
-- `tag` holds the `'ab'`/`'ab '` pair decision-log `[6]`'s review round
-- measured `MIN` diverging over under a padding-sensitive comparison, plus
-- `'ab!'` beside it for the same reason `common/items.rs` keeps a third
-- value: two values alone cannot show which one an aggregate actually picked.
CREATE TABLE items (
    id INTEGER NOT NULL,
    category VARCHAR(20) NOT NULL,
    sort_key VARCHAR(5) NOT NULL,
    tag VARCHAR(10) NOT NULL,
    code CHARACTER(8) NOT NULL,
    description TEXT NOT NULL,
    details JSONB NULL,
    photo BYTEA NULL,
    uid UUID NOT NULL,
    seen_at TIMESTAMPTZ NOT NULL,
    labels TEXT[] NULL,
    PRIMARY KEY (id)
);

INSERT INTO items (id, category, sort_key, tag, code, description, details, photo, uid, seen_at, labels) VALUES
    (1, 'widget', 'B', 'ab ', 'ITEM0001', 'First widget in the fixture.',        '{"tier": "gold"}',   E'\\xDEADBEEF', '00000000-0000-0000-0000-000000000001', '2026-01-01 10:00:00+00', ARRAY['a', 'b']),
    (2, 'Widget', 'a', 'ab',  'ITEM0002', 'Second widget, capitalized.',         NULL,                 E'\\xC0FFEE',   '00000000-0000-0000-0000-000000000002', '2026-01-02 10:00:00+00', NULL),
    (3, 'widget', 'C', 'ab!', 'ITEM0003', 'Third widget, lower case again.',     '{"tier": "silver"}', NULL,           '00000000-0000-0000-0000-000000000003', '2026-01-03 10:00:00+00', ARRAY['c']),
    (4, 'Widget', 'B', 'ab ', 'ITEM0004', 'Fourth widget, capitalized again.',   '{"tier": "silver"}', E'\\xFEED',     '00000000-0000-0000-0000-000000000004', '2026-01-04 10:00:00+00', NULL),
    (5, 'cafe',   'a', 'ab',  'ITEM0005', 'A plain cafe, no accent.',            NULL,                 E'\\xBEEF',     '00000000-0000-0000-0000-000000000005', '2026-01-05 10:00:00+00', ARRAY['d', 'e']),
    (6, 'café',   'C', 'ab!', 'ITEM0006', 'The accented cafe.',                  '{"tier": "gold"}',   E'\\xAB',       '00000000-0000-0000-0000-000000000006', '2026-01-06 10:00:00+00', NULL);

-- `items_c` holds `items`'s four mapped columns and rows with every character
-- column declared `COLLATE "C"`, which sorts and compares by byte value. The
-- adapter renders no collation, so a delegated character comparison, ordering,
-- `GROUP BY`, `COUNT(DISTINCT)`, `MIN`, and `MAX` over this table follow the
-- column's own collation and so return Exasol's code-point answers, which is
-- what lets a differential test compare them with `PG_MIRROR."items_c"`. `items`
-- keeps the database default `en_US.utf8`, whose order the same tests assert
-- as PostgreSQL's own. Rows 1 through 6 are copied from `items` inside
-- PostgreSQL, so the two tables cannot drift apart there. Rows 7 and 8 carry
-- the tags `5%x` and `5\yx`, which separate a `LIKE` pattern whose backslash
-- escapes the `%` from one whose backslash is an ordinary character: only the
-- first row matches the pattern `5\%x` under Exasol's escape rule.
CREATE TABLE items_c (
    id INTEGER NOT NULL,
    category VARCHAR(20) COLLATE "C" NOT NULL,
    sort_key VARCHAR(5) COLLATE "C" NOT NULL,
    tag VARCHAR(10) COLLATE "C" NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO items_c (id, category, sort_key, tag)
    SELECT id, category, sort_key, tag FROM items;

INSERT INTO items_c (id, category, sort_key, tag) VALUES
    (7, 'widget', 'B', '5%x'),
    (8, 'widget', 'B', '5\yx');

-- `item_notes` is `items`'s join partner: `items.id` 1 carries no matching
-- row (unmatched on the `items` side), and `item_id` 99 matches no `items`
-- row (unmatched on the `item_notes` side) — the pair task 8.2's `FULL OUTER
-- JOIN` test needs. Both columns are mapped, so both are mirrored.
CREATE TABLE item_notes (
    item_id INTEGER NOT NULL,
    note VARCHAR(50) NOT NULL,
    PRIMARY KEY (item_id)
);

INSERT INTO item_notes (item_id, note) VALUES
    (2, 'restocked'),
    (3, 'discontinued'),
    (4, 'seasonal'),
    (5, 'restocked'),
    (6, 'discontinued'),
    (99, 'orphan');

-- `numeric_specials` exercises the coercion failure path task 7.1 states
-- rather than a differential row comparison, so it carries no Exasol mirror.
-- Per decision-log gate 2.3 divergence 1: `Infinity`/`-Infinity` are rejected
-- by PostgreSQL itself with "numeric field overflow" when inserted into a
-- *constrained* `numeric(p,s)` column, so `fixed_amount` (mapped to Exasol
-- DECIMAL(12,4)) carries a `NaN` row instead — the one non-finite special a
-- constrained column can hold, and Exasol has no DECIMAL representation for
-- it, which is exactly what makes it end-to-end reachable as a coercion
-- failure rather than a value. `unbounded_amount` is an unconstrained
-- `numeric`, unmapped because it declares no precision
-- (`postgres_type_mapping::map_numeric`), and PostgreSQL accepts all three
-- specials there, so `Infinity` and `-Infinity` are seeded on it instead.
CREATE TABLE numeric_specials (
    id INTEGER NOT NULL,
    fixed_amount NUMERIC(12,4) NOT NULL,
    unbounded_amount NUMERIC NULL,
    PRIMARY KEY (id)
);

INSERT INTO numeric_specials (id, fixed_amount, unbounded_amount) VALUES
    (1, 100.0000, 100),
    (2, 250.5000, NULL),
    (3, 'NaN', 'Infinity'),
    (4, 75.2500, '-Infinity');

-- `all_unmapped` has no column `postgres_type_mapping::map_column` accepts:
-- every column is an array, the one type the mapping leaves out, as the JDBC
-- Virtual Schema does. The omission test asserts the table is absent from
-- `schemaMetadata` entirely, not merely thin. It declares no primary key,
-- because the key would have to be a column of some mapped type.
CREATE TABLE all_unmapped (
    tags TEXT[] NOT NULL,
    numbers INTEGER[] NOT NULL
);

INSERT INTO all_unmapped (tags, numbers) VALUES
    (ARRAY['a', 'b'], ARRAY[1, 2]),
    (ARRAY['c'],      ARRAY[3]);

-- `type_coverage` holds one column of every PostgreSQL type the mapping maps
-- beyond the core types above, so a differential test reaches each conversion
-- through `crates/adbc-vs-adapter/tests/common/postgres_mirror.rs`, which holds
-- the values Exasol reads: a converted column as PostgreSQL's text, `money`
-- and `real` as `DOUBLE`, the `bit` columns as `BOOLEAN`, and each timestamp
-- at its declared precision. Keep the two in sync by hand if either changes.
--
-- Rows separate the semantics a conversion could blur:
-- `unbounded_amount` holds `1.0` and `1.00`, two distinct texts for one
-- number, and `NaN`, `Infinity`, and `1e400`, which no DECIMAL holds.
-- `uid` is lower case, so an upper-case spelling of it selects nothing once it
-- is compared as text. `doc_json` keeps its irregular whitespace and
-- `doc_jsonb` reorders keys, so each compares as the text PostgreSQL prints.
-- `flag`, `other_flag`, and `bit_flag` hold true, false, and null and differ in
-- at least one row. `bits` holds `00000101`, `00001001`, and `00000000`, so two
-- different values read as one `true`. `fixed_code` holds `ab` and `abc` beside
-- `label` holding `ab`, which a `character(5)` and a `character varying(5)`
-- compare equal on in PostgreSQL and unequal on in Exasol. `wide_code` is wider
-- than Exasol's `CHAR`, so it is read as text without its blanks.
-- `happened_at` carries explicit `+00` offsets, and `local_at` holds its UTC
-- wall-clock value in some rows and another value in others, so a delegated
-- equality of the two is true in some rows and false in others.
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

CREATE TABLE type_coverage (
    id INTEGER NOT NULL,
    small_id SMALLINT NOT NULL,
    unbounded_amount NUMERIC NULL,
    wide_amount NUMERIC(40,2) NULL,
    uid UUID NULL,
    doc_json JSON NULL,
    doc_jsonb JSONB NULL,
    doc_xml XML NULL,
    raw BYTEA NULL,
    span INTERVAL NULL,
    host INET NULL,
    price MONEY NULL,
    approx REAL NULL,
    flag BOOLEAN NULL,
    other_flag BOOLEAN NULL,
    bit_flag BIT(1) NULL,
    bits BIT(8) NULL,
    varbits BIT VARYING(16) NULL,
    fixed_code CHARACTER(5) NULL,
    label VARCHAR(5) NULL,
    wide_code CHARACTER(3000) NULL,
    feeling mood NULL,
    clock TIME(3) NULL,
    clock_tz TIME WITH TIME ZONE NULL,
    precise_at TIMESTAMP(6) NULL,
    whole_at TIMESTAMP(0) NULL,
    happened_at TIMESTAMPTZ NULL,
    local_at TIMESTAMP(6) NULL,
    PRIMARY KEY (id)
);

INSERT INTO type_coverage (
    id, small_id, unbounded_amount, wide_amount, uid, doc_json, doc_jsonb, doc_xml, raw, span, host,
    price, approx, flag, other_flag, bit_flag, bits, varbits, fixed_code, label, wide_code, feeling,
    clock, clock_tz, precise_at, whole_at, happened_at, local_at
) VALUES
    (1, 1, '1.0', 12345678901234567890123456789012345678.90, 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11',
     '{"a":   1,  "b": [1,2]}', '{"b": 1, "a": 2}', '<a>1</a>', E'\\xDEADBEEF', '1 day 02:03:04', '192.168.0.1',
     12.34, 0.1, TRUE, TRUE, B'1', B'00000101', B'1010', 'ab', 'ab', 'ab', 'ok',
     '16:59:59.123', '16:59:59+02', '2024-01-01 12:00:00.123456', '2024-01-01 12:00:01',
     '2024-01-01 12:00:00.123456+00', '2024-01-01 12:00:00.123456'),
    (2, 2, '1.00', NULL, 'b0eebc99-9c0b-4ef8-bb6d-6bb9bd380a22',
     '{"x": null}', '{"a": 2, "b": 1}', '<b/>', E'\\xC0FFEE', '2 hours', '10.0.0.1/8',
     0.10, 0.5, FALSE, TRUE, B'0', B'00001001', B'1', 'abc', 'ab', NULL, 'happy',
     '00:00:01', '08:00:00-05', '2024-01-01 12:00:00.123457', '2024-06-01 00:00:00',
     '2024-01-02 12:00:00+00', '2024-01-03 12:00:00'),
    (3, 3, 'NaN', NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, B'00000000', NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, NULL),
    (4, 4, 'Infinity', NULL, 'c0eebc99-9c0b-4ef8-bb6d-6bb9bd380a33',
     NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, TRUE, FALSE, B'1', B'00000101', NULL, 'ab', 'ab', 'ab', 'sad',
     NULL, NULL, '2024-01-01 12:00:00.123456', NULL, '2024-01-04 12:00:00+00', '2024-01-04 12:00:00'),
    (5, 5, '1e400', NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, FALSE, FALSE, B'0', NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL, '2024-01-05 12:00:00+00', '2024-01-06 12:00:00');

-- `text_overflow` holds one `text` value one character longer than Exasol's
-- `VARCHAR` maximum of 2000000, so a query reading it must fail rather than
-- return the value truncated.
CREATE TABLE text_overflow (
    id INTEGER NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO text_overflow (id, body) VALUES (1, repeat('x', 2000001));

-- `collated_names` holds one `character varying` column under a
-- nondeterministic collation that equates two values differing only by case,
-- which PostgreSQL's built-in collations never do. An operator creates such a
-- collation, so the column must map like any other and compare as PostgreSQL
-- compares it: `Widget` and `widget` are one value to PostgreSQL and two to
-- Exasol.
CREATE COLLATION case_insensitive (
    provider = icu,
    locale = 'und-u-ks-level2',
    deterministic = false
);

CREATE TABLE collated_names (
    id INTEGER NOT NULL,
    name VARCHAR(20) COLLATE case_insensitive NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO collated_names (id, name) VALUES
    (1, 'Widget'),
    (2, 'widget'),
    (3, 'Gadget');

-- End-to-end fixture for the PostgreSQL tier. Each object
-- below is dedicated to exactly one end-to-end test, so no other test's
-- assertions are disturbed by what that test does to it.

-- `refresh_target` is the source `refresh_picks_up_a_new_postgresql_column`
-- alters mid-run (`ADD COLUMN` then `DROP COLUMN`). It carries no columns any
-- other test reads, so that test's DDL cannot poison a differential
-- comparison another test relies on.
CREATE TABLE refresh_target (
    id INTEGER NOT NULL,
    name VARCHAR(30) NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO refresh_target (id, name) VALUES
    (1, 'Original');

-- `order_names` is `views_are_exposed_alongside_base_tables`'s subject: a
-- view over `orders`, projecting only its mapped `id` and `name` columns, so
-- the exposed table carries exactly two columns and no `omitted` entry.
CREATE VIEW order_names AS SELECT id, name FROM orders;

-- `"MixedCase"` is `identifiers_keep_their_case_through_discovery_and_rendering`'s
-- subject. Every identifier — the table name and both column names — is
-- quoted, because PostgreSQL folds an unquoted identifier to lower case,
-- which would make the fixture indistinguishable from case folding rather
-- than proof of case preservation.
CREATE TABLE "MixedCase" (
    "ItemId" INTEGER NOT NULL,
    "Label" VARCHAR(20) NOT NULL,
    PRIMARY KEY ("ItemId")
);

INSERT INTO "MixedCase" ("ItemId", "Label") VALUES
    (1, 'First'),
    (2, 'Second');
