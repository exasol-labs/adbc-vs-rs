-- Core-type fixture for the PostgreSQL integration tier (`postgres-it`).
-- Column set matches `postgres_type_mapping.rs`'s seven mapped types:
-- `integer`, `bigint`, `numeric(p,s)`, `character varying(n)`, `date`,
-- `timestamp without time zone`, and `double precision`.
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
-- `items` mixes mapped and unmapped columns on purpose, so task 8.4's
-- omission test has a table that is *partly* mapped rather than only the
-- all-or-nothing cases `orders` and `all_unmapped` cover. Its mapped columns
-- (`id`, `category`, `sort_key`, `tag`) are the ones
-- `crates/adbc-vs-adapter/tests/common/postgres_mirror.rs` reproduces in
-- Exasol under schema `PG_MIRROR`, table `items`, seeded from byte-identical
-- literals — keep the two in sync by hand if either changes, the same
-- obligation `common/items.rs`'s header states for the MySQL fixture. Its
-- unmapped columns (`code`, `description`, `details`, `photo`, `uid`,
-- `seen_at`) have no mirror counterpart: metadata discovery unmaps every one
-- of them (`character`, `text`, `jsonb`, `bytea`, `uuid`, `timestamptz` all
-- fail `postgres_type_mapping::map_data_type`), so no delegated query can
-- reach them and a mirror column would be compared against nothing — the
-- same reasoning `common/mirror.rs` states for MySQL's latin1 `region`
-- column.
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
    PRIMARY KEY (id)
);

INSERT INTO items (id, category, sort_key, tag, code, description, details, photo, uid, seen_at) VALUES
    (1, 'widget', 'B', 'ab ', 'ITEM0001', 'First widget in the fixture.',        '{"tier": "gold"}',   E'\\xDEADBEEF', '00000000-0000-0000-0000-000000000001', '2026-01-01 10:00:00+00'),
    (2, 'Widget', 'a', 'ab',  'ITEM0002', 'Second widget, capitalized.',         NULL,                 E'\\xC0FFEE',   '00000000-0000-0000-0000-000000000002', '2026-01-02 10:00:00+00'),
    (3, 'widget', 'C', 'ab!', 'ITEM0003', 'Third widget, lower case again.',     '{"tier": "silver"}', NULL,           '00000000-0000-0000-0000-000000000003', '2026-01-03 10:00:00+00'),
    (4, 'Widget', 'B', 'ab ', 'ITEM0004', 'Fourth widget, capitalized again.',   '{"tier": "silver"}', E'\\xFEED',     '00000000-0000-0000-0000-000000000004', '2026-01-04 10:00:00+00'),
    (5, 'cafe',   'a', 'ab',  'ITEM0005', 'A plain cafe, no accent.',            NULL,                 E'\\xBEEF',     '00000000-0000-0000-0000-000000000005', '2026-01-05 10:00:00+00'),
    (6, 'café',   'C', 'ab!', 'ITEM0006', 'The accented cafe.',                  '{"tier": "gold"}',   E'\\xAB',       '00000000-0000-0000-0000-000000000006', '2026-01-06 10:00:00+00');

-- `items_c` holds `items`'s four mapped columns and rows with every character
-- column declared `COLLATE "C"`, which sorts and compares by byte value. The
-- adapter renders no collation, so a delegated character comparison, ordering,
-- `GROUP BY`, `COUNT(DISTINCT)`, `MIN`, and `MAX` over this table follow the
-- column's own collation and so return Exasol's code-point answers, which is
-- what lets a differential test compare them with `PG_MIRROR."items"`. `items`
-- keeps the database default `en_US.utf8`, whose order the same tests assert
-- as PostgreSQL's own. The rows are copied from `items` inside PostgreSQL, so
-- the two tables cannot drift apart.
CREATE TABLE items_c (
    id INTEGER NOT NULL,
    category VARCHAR(20) COLLATE "C" NOT NULL,
    sort_key VARCHAR(5) COLLATE "C" NOT NULL,
    tag VARCHAR(10) COLLATE "C" NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO items_c (id, category, sort_key, tag)
    SELECT id, category, sort_key, tag FROM items;

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

-- `all_unmapped` has no column `postgres_type_mapping::map_column` accepts —
-- every one of its fifteen columns is one of the reasons
-- `postgres_type_mapping.rs` names: `character`, `boolean`, `text`,
-- `smallint`, `real`, both `time` types, `timestamptz`, `json`, `jsonb`,
-- `bytea`, `interval`, `money`, and `ARRAY`. `id` is `UUID` rather than an
-- integer type, because every mapped type is exactly what this table must
-- not declare even for its primary key. Task 8.4's omission test asserts the
-- table is absent from `schemaMetadata` entirely, not merely thin.
CREATE TABLE all_unmapped (
    id UUID NOT NULL,
    fixed_code CHARACTER(4) NOT NULL,
    is_active BOOLEAN NOT NULL,
    notes TEXT NOT NULL,
    small_num SMALLINT NOT NULL,
    approx REAL NOT NULL,
    clock_time TIME NOT NULL,
    clock_time_tz TIME WITH TIME ZONE NOT NULL,
    seen_at TIMESTAMPTZ NOT NULL,
    payload JSON NOT NULL,
    payload_b JSONB NOT NULL,
    raw_data BYTEA NOT NULL,
    span INTERVAL NOT NULL,
    price MONEY NOT NULL,
    tags TEXT[] NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO all_unmapped (id, fixed_code, is_active, notes, small_num, approx, clock_time, clock_time_tz, seen_at, payload, payload_b, raw_data, span, price, tags) VALUES
    ('a1111111-1111-1111-1111-111111111111', 'ABCD', TRUE,  'first row of the all-unmapped fixture',  7,  3.14, '12:00:00', '12:00:00+02', '2026-01-01 00:00:00+00', '{"a": 1}', '{"b": 2}', E'\\xDEADBEEF', '1 day',    19.99, ARRAY['a', 'b']),
    ('a2222222-2222-2222-2222-222222222222', 'WXYZ', FALSE, 'second row of the all-unmapped fixture', 42, 2.71, '18:30:00', '18:30:00-05', '2026-02-02 00:00:00+00', '{"c": 3}', '{"d": 4}', E'\\xC0FFEE',   '2 hours', 5.00,  ARRAY['c']);

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
