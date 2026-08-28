-- Core-type fixture for the mysql-roundtrip end-to-end test.
-- Column set matches plan.md's "Manual Testing" row for type-mapping:
-- DECIMAL(10,0), DECIMAL(19,0), DECIMAL(12,4), VARCHAR(100), CHAR(8), DATE, TIMESTAMP, DOUBLE.
-- `id` and `big_id` are signed integer types that map to DECIMAL(10,0)/DECIMAL(19,0);
-- `amount` is a native DECIMAL(12,4) column, seeded with a NULL and a maximum-scale
-- value so the Arrow decimal-coercion path is exercised end to end.
-- `created_at` is DATETIME, not TIMESTAMP: DATETIME is the MySQL type release 0.1
-- maps to Exasol TIMESTAMP, and a TIMESTAMP column would be omitted as unmapped.

CREATE TABLE orders (
    id INT NOT NULL,
    big_id BIGINT NOT NULL,
    amount DECIMAL(12,4) NULL,
    name VARCHAR(100) NOT NULL,
    code CHAR(8) NOT NULL,
    order_date DATE NOT NULL,
    created_at DATETIME NOT NULL,
    price DOUBLE NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO orders (id, big_id, amount, name, code, order_date, created_at, price) VALUES
    (1, 9000000001, 1234.5600,     'Widget', 'CODE0001', '2026-01-15', '2026-01-15 08:30:00', 19.99),
    (2, 9000000002, NULL,          'Gadget', 'CODE0002', '2026-02-20', '2026-02-20 14:45:00', 42.5),
    (3, 9000000003, 99999999.9999, 'Gizmo',  'CODE0003', '2026-03-25', '2026-03-25 23:59:59', 0.01);
