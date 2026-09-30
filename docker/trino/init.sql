-- Core-type fixture for the Trino integration and E2E tiers, mirroring
-- docker/postgres/init.sql's initial `orders` table over Trino's own type
-- spellings, against the built-in `memory` catalog. It carries two halves:
-- the core-type `orders` table the integration tier reads, and the
-- differential objects the E2E tier reads -- `items`, `item_notes`,
-- `all_unmapped`, `refresh_target`, and `orders_view` -- which cover
-- collation, joins, a view, and unmapped columns respectively.
--
-- `created_at_ns` adds a timestamp(9) column beyond the core set every
-- other source's fixture already covers, because precision 9 is the only
-- one any driver this release supports returns Arrow `timestamp[ns]` for.
--
-- The four columns beside it exist for the same reason: this driver is the
-- only one that picks an Arrow width and unit from the declared precision
-- rather than returning one of each. Gate 2.4 measured the boundaries, and
-- these columns are one fixture per measured band, so no band is asserted
-- from a column that shares another's representation.
--   small_amount  decimal(9,2)   -> Decimal32   (precision up to 9)
--   amount        decimal(12,4)  -> Decimal64   (precision 10 to 18)
--   large_amount  decimal(24,4)  -> Decimal128  (precision 19 up)
--   created_at_s  timestamp(0)   -> timestamp[s]
--   created_at_ms timestamp(3)   -> timestamp[ms]
--   created_at    timestamp(6)   -> timestamp[us]
--   created_at_ns timestamp(9)   -> timestamp[ns]
-- The third row seeds each numeric column at its own maximum, so a width
-- that silently narrowed would overflow rather than round-trip.
--
-- The `memory` connector holds no data across a container restart, so this
-- file is re-applied by `stack-up-trino` on every stack start, through the
-- container's own CLI (`--file`) -- the image mounts no
-- docker-entrypoint-initdb.d-style directory the way postgres/mysql do.
-- `stack-up-trino` also re-runs this same invocation as its own readiness
-- poll (see the Makefile), so it must equally tolerate a container that was
-- already up and already seeded from a prior run -- `DROP TABLE IF EXISTS`
-- makes that re-run idempotent instead of failing on "table already exists".

CREATE SCHEMA IF NOT EXISTS memory.adbc_vs;

DROP TABLE IF EXISTS memory.adbc_vs.orders;

CREATE TABLE memory.adbc_vs.orders (
    id INTEGER,
    big_id BIGINT,
    small_amount DECIMAL(9,2),
    amount DECIMAL(12,4),
    large_amount DECIMAL(24,4),
    name VARCHAR(100),
    order_date DATE,
    created_at_s TIMESTAMP(0),
    created_at_ms TIMESTAMP(3),
    created_at TIMESTAMP(6),
    created_at_ns TIMESTAMP(9),
    price DOUBLE
);

INSERT INTO memory.adbc_vs.orders
    (id, big_id, small_amount, amount, large_amount, name, order_date, created_at_s, created_at_ms, created_at, created_at_ns, price)
VALUES
    (1, 9000000001, 1234.56,     1234.5600,     12345678901234567890.1234,  'Widget', DATE '2026-01-15', TIMESTAMP '2026-01-15 08:30:00', TIMESTAMP '2026-01-15 08:30:00.123', TIMESTAMP '2026-01-15 08:30:00.123456', TIMESTAMP '2026-01-15 08:30:00.123456789', 19.99),
    (2, 9000000002, -9999999.99, CAST(NULL AS DECIMAL(12,4)), -12345678901234567890.1234, 'Gadget', DATE '2026-02-20', TIMESTAMP '2026-02-20 14:45:00', TIMESTAMP '2026-02-20 14:45:00.001', TIMESTAMP '2026-02-20 14:45:00.000001', TIMESTAMP '2026-02-20 14:45:00.000000001', 42.5),
    (3, 9000000003, 9999999.99,  99999999.9999, 99999999999999999999.9999,  'Gizmo',  DATE '2026-03-25', TIMESTAMP '2026-03-25 23:59:59', TIMESTAMP '2026-03-25 23:59:59.999', TIMESTAMP '2026-03-25 23:59:59.999999', TIMESTAMP '2026-03-25 23:59:59.999999999', 0.01);

-- Differential end-to-end fixture (task 8.1). `orders` above is left exactly
-- as task 5.4 (group E) seeded it -- the four numeric-width and
-- timestamp-precision columns the coercion scenario binds to stay
-- untouched -- and every object below is new.
--
-- `items` mixes mapped and unmapped columns on purpose, mirroring
-- `docker/postgres/init.sql`'s own `items` table over Trino's spellings.
-- Its mapped columns (`id`, `category`, `sort_key`, `tag`) are the ones
-- `crates/adbc-vs-adapter/tests/common/trino_mirror.rs` reproduces in
-- Exasol under schema `TR_MIRROR`, table `items`, seeded from
-- byte-identical literals -- keep the two in sync by hand if either
-- changes, the same obligation `docker/postgres/init.sql`'s header states.
-- Its unmapped columns (`code`, `bio`, `active`) are each one of the
-- reasons `trino_type_mapping.rs` names: `char(n)` is blank-padded and
-- compares differently under `LIKE` than under `=`, a bare `varchar` has no
-- bounded Exasol width, and no mapped column produces Exasol `BOOLEAN`.
--
-- `category` holds the exact case-only/accent-only set gate 2.3(a) measured
-- Trino's code-point comparison agreeing with Exasol's own on: `widget` x2,
-- `Widget` x2, `cafe` x1, `café` x1.
-- `sort_key` holds the `B`/`a`/`C` triple `vs-adapter/trino-collation`'s own
-- scenario names, seeded out of sorted order so a delegated
-- `ORDER BY`/`MIN`/`MAX` cannot pass on storage order alone.
-- `tag` holds the `'ab'`/`'ab '` pair the trailing-space `MIN`/`MAX`
-- scenario needs, plus `'ab!'` beside it so no aggregate answer can be
-- inferred from only two values.
DROP TABLE IF EXISTS memory.adbc_vs.items;

CREATE TABLE memory.adbc_vs.items (
    id INTEGER,
    category VARCHAR(20),
    sort_key VARCHAR(5),
    tag VARCHAR(10),
    code CHAR(8),
    bio VARCHAR,
    active BOOLEAN
);

INSERT INTO memory.adbc_vs.items (id, category, sort_key, tag, code, bio, active) VALUES
    (1, 'widget', 'B', 'ab ', 'ITEM0001', 'First widget in the fixture.',      true),
    (2, 'Widget', 'a', 'ab',  'ITEM0002', 'Second widget, capitalized.',       false),
    (3, 'widget', 'C', 'ab!', 'ITEM0003', 'Third widget, lower case again.',   true),
    (4, 'Widget', 'B', 'ab ', 'ITEM0004', 'Fourth widget, capitalized again.', false),
    (5, 'cafe',   'a', 'ab',  'ITEM0005', 'A plain cafe, no accent.',          true),
    (6, 'café',   'C', 'ab!', 'ITEM0006', 'The accented cafe.',                false);

-- `item_notes` is `items`'s join partner: `items.id` 1 carries no matching
-- row (unmatched on the `items` side), and `item_id` 99 matches no `items`
-- row (unmatched on the `item_notes` side) -- the pair the full outer join
-- test needs. Both columns are mapped, so both are mirrored.
DROP TABLE IF EXISTS memory.adbc_vs.item_notes;

CREATE TABLE memory.adbc_vs.item_notes (
    item_id INTEGER,
    note VARCHAR(50)
);

INSERT INTO memory.adbc_vs.item_notes (item_id, note) VALUES
    (2, 'restocked'),
    (3, 'discontinued'),
    (4, 'seasonal'),
    (5, 'restocked'),
    (6, 'discontinued'),
    (99, 'orphan');

-- `all_unmapped` has no column `trino_type_mapping::map_data_type` accepts.
-- `id` is `UUID` rather than an integer type, because every mapped type is
-- exactly what this table must not declare even for its own identity
-- column. The remaining columns cover every unmapped spelling gate 2.2 and
-- gate 2.4 measured beyond the ones `items` above already exercises:
-- `smallint`, `tinyint`, `real`, both `time` forms, `timestamp(p) with time
-- zone`, `varbinary`, `json`, `array`, `map`, `row`, and `interval`.
DROP TABLE IF EXISTS memory.adbc_vs.all_unmapped;

CREATE TABLE memory.adbc_vs.all_unmapped (
    id UUID,
    small_num SMALLINT,
    tiny_num TINYINT,
    approx REAL,
    clock_time TIME(3),
    clock_time_tz TIME(3) WITH TIME ZONE,
    seen_at TIMESTAMP(3) WITH TIME ZONE,
    raw_data VARBINARY,
    payload JSON,
    tags ARRAY(INTEGER),
    attributes MAP(VARCHAR, INTEGER),
    coords ROW(x INTEGER, y INTEGER),
    span INTERVAL DAY TO SECOND
);

INSERT INTO memory.adbc_vs.all_unmapped
    (id, small_num, tiny_num, approx, clock_time, clock_time_tz, seen_at, raw_data, payload, tags, attributes, coords, span)
VALUES
    (UUID 'a1111111-1111-1111-1111-111111111111', 7, 1, 3.14, TIME '12:00:00', TIME '12:00:00 +02:00', TIMESTAMP '2026-01-01 00:00:00 UTC', X'DEADBEEF', JSON '{"a": 1}', ARRAY[1, 2], MAP(ARRAY['a'], ARRAY[1]), ROW(1, 2), INTERVAL '1' DAY),
    (UUID 'a2222222-2222-2222-2222-222222222222', 42, 2, 2.71, TIME '18:30:00', TIME '18:30:00 -05:00', TIMESTAMP '2026-02-02 00:00:00 UTC', X'C0FFEE', JSON '{"c": 3}', ARRAY[3], MAP(ARRAY['b'], ARRAY[2]), ROW(3, 4), INTERVAL '2' HOUR);

-- `refresh_target` is the source `refresh_picks_up_a_new_trino_column`
-- alters mid-run (`ADD COLUMN` then `DROP COLUMN`). It carries no columns
-- any other test reads, so that test's DDL cannot poison a differential
-- comparison another test relies on.
DROP TABLE IF EXISTS memory.adbc_vs.refresh_target;

CREATE TABLE memory.adbc_vs.refresh_target (
    id INTEGER,
    name VARCHAR(30)
);

INSERT INTO memory.adbc_vs.refresh_target (id, name) VALUES (1, 'Original');

-- `orders_view` is a view over `orders`, projecting only its mapped `id`
-- and `name` columns -- `views_are_exposed_alongside_base_tables`'s
-- subject, mirroring `docker/postgres/init.sql`'s own `order_names`.
--
-- No mixed-case fixture object is added here the way
-- `docker/postgres/init.sql`'s `"MixedCase"` is: measured directly against
-- this container, Trino's `memory` connector folds every table and column
-- name to lower case regardless of how the DDL that created it quoted the
-- identifier, so a quoted mixed-case name here would prove nothing about
-- case preservation -- there is no case left for the source to preserve by
-- the time discovery reads it back. `e2e_trino_test.rs`'s own
-- identifier-case test instead asserts that discovery and rendering keep
-- whatever case Trino's own catalog reports, byte for byte.
DROP VIEW IF EXISTS memory.adbc_vs.orders_view;

CREATE VIEW memory.adbc_vs.orders_view AS SELECT id, name FROM memory.adbc_vs.orders;
