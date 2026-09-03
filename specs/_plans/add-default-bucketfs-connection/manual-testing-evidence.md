# Manual Testing Evidence: add-default-bucketfs-connection

Phase 5c of `/speq:implement` — manual verification against the live, already-built stack. Plan section: `specs/_plans/add-default-bucketfs-connection/plan.md` § Verification / Manual Testing (lines 356-367).

## Environment note

This worktree's plan and code live in `/home/talos/code/labs-adbc-vs-4` (branch `feat/add-default-bucketfs-connection`, HEAD `a5dcced`), not in `/home/talos/code/labs-adbc-vs` (`main`) where this task was initially framed — `main` has no such plan and no such code (`BUCKETFS_CONNECTION` is still a hard-required property there). All commands below were run against the already-running `labs-adbc-vs-4` Compose stack (`labs-adbc-vs-4-exasol-1`, `-mysql-1`, `-postgres-1`, all `Up`), against the already-built artifact `target/release/libadbc_vs_adapter.so` in that worktree, confirmed newer than every source file (no stale-binary risk). `make cross-udf-build && make stack-up` was skipped per instructions, since the stack was already up.

```
DSN="exasol://sys:exasol@localhost:8563?validateservercertificate=0"
MOUNT="/buckets/bfsdefault/default"
```

Container's own network address: `172.28.5.2`. BucketFS write password read from `/exa/etc/EXAConf` inside the container (base64-decoded), used to reconstruct the exact `ADBC_VS_BUCKETFS` connection statement per `crates/adbc-vs-adapter/tests/common/schema.rs::create_bucketfs_connection`.

---

## Row 1 — `create-virtual-schema`: default connection

**PASS**

```
$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA DEFVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
[1/1] CREATE VIRTUAL SCHEMA DEFVS USING ADBC_VS.ADBC_ADAPTER WI... OK
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM DEFVS."orders"'
[1/1] SELECT COUNT(*) FROM DEFVS."orders" 1 rows
COUNT(*)
3
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" "SELECT PROPERTY_NAME, PROPERTY_VALUE FROM SYS.EXA_ALL_VIRTUAL_SCHEMA_PROPERTIES WHERE SCHEMA_NAME='DEFVS'"
[1/1] SELECT PROPERTY_NAME, PROPERTY_VALUE FROM SYS.EXA_ALL_VIR... 2 rows
PROPERTY_NAME,PROPERTY_VALUE
DRIVER,mysql
CONNECTION_NAME,ADBC_VS_MYSQL
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" 'DROP VIRTUAL SCHEMA DEFVS CASCADE'
[1/1] DROP VIRTUAL SCHEMA DEFVS CASCADE OK
1 statement executed, 0 failed
```

Matches expectation exactly: schema created with only `DRIVER` and `CONNECTION_NAME` properties recorded — no `BUCKETFS_CONNECTION` — and the count is `3`.

---

## Row 2 — `create-virtual-schema`: explicit override never falls back

**PASS**

```
$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA BADVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL' BUCKETFS_CONNECTION = 'ADBC_VS_NO_SUCH_CONNECTION'"
Error in statement 1:
  CREATE VIRTUAL SCHEMA BADVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL' BUCKETFS_CONNECTION = 'ADBC_VS_NO_SUCH_CONNECTION'

  Query execution failed: Protocol error: VM error: F-UDF-CL-RUST-9001: UDF error: UDF error: resolving CONNECTION `ADBC_VS_NO_SUCH_CONNECTION`: Connect-back error: Protocol error: Protocol error: MT_IMPORT response missing connection_information: Connect-back error: Protocol error: Protocol error: MT_IMPORT response missing connection_information (Session: 1875328401192714240) (SQL state: 22002)

  Hint: Check the connection string and ensure the database is reachable.
1 statement executed, 1 failed

$ exapump sql --dsn "$DSN" "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='BADVS'"
[1/1] SELECT SCHEMA_NAME FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE... 0 rows
SCHEMA_NAME
1 statement executed, 0 failed
```

Fails naming `ADBC_VS_NO_SUCH_CONNECTION` and no other connection name; no schema created (0 rows for `BADVS`).

---

## Row 3 — `driver-path-override`: no route at all

**PASS**

```
$ exapump sql --dsn "$DSN" 'DROP CONNECTION ADBC_VS_BUCKETFS'
[1/1] DROP CONNECTION ADBC_VS_BUCKETFS OK
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA NOROUTEVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
Error in statement 1:
  CREATE VIRTUAL SCHEMA NOROUTEVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'

  Query execution failed: Protocol error: VM error: F-UDF-CL-RUST-9001: UDF error: UDF error: no `mysql` driver library is available: the statement named neither BUCKETFS_CONNECTION nor DRIVER_PATH, so the reserved CONNECTION `ADBC_VS_BUCKETFS` was the route to a self-installed one. That reserved connection did not resolve (resolving CONNECTION `ADBC_VS_BUCKETFS`: Connect-back error: Protocol error: Protocol error: MT_IMPORT response missing connection_information). The default driver location /buckets/bfsdefault/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so holds no library this adapter can load either (No such file or directory (os error 2)), so no route is left. Pre-place the `mysql` library there, name one with DRIVER_PATH, or name a BucketFS write credential with BUCKETFS_CONNECTION.: Connect-back error: Protocol error: Protocol error: MT_IMPORT response missing connection_information (Session: 1875328409349652480) (SQL state: 22002)

  Hint: Check the connection string and ensure the database is reachable.
1 statement executed, 1 failed

$ exapump sql --dsn "$DSN" "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='NOROUTEVS'"
[1/1] SELECT SCHEMA_NAME FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE... 0 rows
SCHEMA_NAME
1 statement executed, 0 failed
```

The message names all four required subjects: `$MOUNT/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`, `ADBC_VS_BUCKETFS`, `BUCKETFS_CONNECTION`, and `DRIVER_PATH`. No schema created.

---

## Row 4 — `driver-path-override`: pre-placed library, no connection

**PASS**

```
$ exapump bucketfs ls -r adbc-drivers
postgresql/1.12.0/libadbc_driver_postgresql.so
mysql/0.6.0/libadbc_driver_mysql.so

$ exapump bucketfs cp adbc-drivers/mysql/0.6.0/libadbc_driver_mysql.so /tmp/.../libadbc_driver_mysql.so
Downloaded adbc-drivers/mysql/0.6.0/libadbc_driver_mysql.so to /tmp/.../libadbc_driver_mysql.so   (46408368 bytes)

$ exapump bucketfs cp /tmp/.../libadbc_driver_mysql.so adbc-drivers/mysql/supplied/libadbc_driver_mysql.so --bfs-write-password "<redacted>"
Uploaded /tmp/.../libadbc_driver_mysql.so to adbc-drivers/mysql/supplied/libadbc_driver_mysql.so
```

Polled until `docker exec labs-adbc-vs-4-exasol-1 test -f /exa/data/bucketfs/bfsdefault/.dest/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so` succeeded (readable to a UDF), then:

```
$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA PREVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
[1/1] CREATE VIRTUAL SCHEMA PREVS USING ADBC_VS.ADBC_ADAPTER WI... OK
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM PREVS."orders"'
[1/1] SELECT COUNT(*) FROM PREVS."orders" 1 rows
COUNT(*)
3
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'PREVS'"
ADAPTER_NOTES
"{""driver"":""mysql"",""driverPath"":""/buckets/bfsdefault/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so"",""connection"":""ADBC_VS_MYSQL"",""database"":""adbc_vs"", ...omitted-columns...}"
```

`driverPath` records the `supplied` path; no `driverVersion` key present in the JSON. Count is `3`.

Cleanup performed:

```
$ exapump sql --dsn "$DSN" 'DROP VIRTUAL SCHEMA PREVS CASCADE'
[1/1] DROP VIRTUAL SCHEMA PREVS CASCADE OK

$ exapump sql --dsn "$DSN" "CREATE OR REPLACE CONNECTION ADBC_VS_BUCKETFS TO 'https://172.28.5.2:2581/default' USER 'w' IDENTIFIED BY '<redacted>'"
[1/1] CREATE OR REPLACE CONNECTION ADBC_VS_BUCKETFS TO 'https:/... OK
```

(Restore statement reconstructed from `crates/adbc-vs-adapter/tests/common/schema.rs::create_bucketfs_connection`/`connection_statement`, using the container's own Compose-network address for the BucketFS write URL and the write password read out of `/exa/etc/EXAConf` inside the container — exactly what the E2E harness uses.)

---

## Row 5 — `driver-path-override`: pre-placed library, unreachable registry

**PASS** (iptables available in the container image — not skipped)

```
$ docker exec labs-adbc-vs-4-exasol-1 which iptables
/usr/sbin/iptables

$ docker exec labs-adbc-vs-4-exasol-1 iptables -A OUTPUT -p tcp --dport 443 -j DROP
$ docker exec labs-adbc-vs-4-exasol-1 iptables -L OUTPUT
Chain OUTPUT (policy ACCEPT)
target     prot opt source               destination
DROP       tcp  --  anywhere             anywhere             tcp dpt:https

$ timeout 8 docker exec labs-adbc-vs-4-exasol-1 curl -s -o /dev/null -w "%{http_code}\n" --max-time 5 https://cdn.jsdelivr.net/
000        (exit 28 — connect timeout; egress on 443 confirmed blocked)
```

```
$ exapump bucketfs rm adbc-drivers/mysql/0.6.0/libadbc_driver_mysql.so --bfs-write-password "<redacted>"
Deleted adbc-drivers/mysql/0.6.0/libadbc_driver_mysql.so
```

Polled until the versioned object was gone from the container's unpacked bucket path, then:

```
$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA OFFLINEVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
[1/1] CREATE VIRTUAL SCHEMA OFFLINEVS USING ADBC_VS.ADBC_ADAPTE... OK
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM OFFLINEVS."orders"'
COUNT(*)
3

$ exapump sql --dsn "$DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'OFFLINEVS'"
ADAPTER_NOTES
"{""driver"":""mysql"",""driverPath"":""/buckets/bfsdefault/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so"",""connection"":""ADBC_VS_MYSQL"",""database"":""adbc_vs"", ...omitted-columns...}"
```

`driverPath` records the `supplied` path with no `driverVersion` — same as row 4, but this time from a real network failure against the live CDN (not the stubbed cases `provisioning_test.rs` exercises), with the reserved `ADBC_VS_BUCKETFS` connection present and resolvable. This is the one row the plan's own coverage note (plan.md line 352) says no automated tier reaches.

Cleanup performed:

```
$ exapump sql --dsn "$DSN" 'DROP VIRTUAL SCHEMA OFFLINEVS CASCADE'
[1/1] DROP VIRTUAL SCHEMA OFFLINEVS CASCADE OK

$ docker exec labs-adbc-vs-4-exasol-1 iptables -D OUTPUT -p tcp --dport 443 -j DROP
$ docker exec labs-adbc-vs-4-exasol-1 iptables -L OUTPUT
Chain OUTPUT (policy ACCEPT)
target     prot opt source               destination
                                                          (empty — rule removed)

$ timeout 8 docker exec labs-adbc-vs-4-exasol-1 curl -s -o /dev/null -w "%{http_code}\n" --max-time 5 https://cdn.jsdelivr.net/
301        (exit 0 — egress confirmed restored)
```

---

## Row 6 — `driver-self-install`: the supplied file is never written

**PASS**

Pre-state confirmed: connection restored (row 4/5 cleanup), supplied copy still present, versioned object still deleted (row 5 cleanup):

```
$ exapump bucketfs ls -r adbc-drivers/mysql
supplied/libadbc_driver_mysql.so
```

```
$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA REINSTALLVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
[1/1] CREATE VIRTUAL SCHEMA REINSTALLVS USING ADBC_VS.ADBC_ADAP... OK
1 statement executed, 0 failed

$ exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM REINSTALLVS."orders"'
COUNT(*)
3

$ exapump sql --dsn "$DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'REINSTALLVS'"
ADAPTER_NOTES
"{""driver"":""mysql"",""driverVersion"":""0.6.0"",""driverPath"":""/buckets/bfsdefault/default/adbc-drivers/mysql/0.6.0/libadbc_driver_mysql.so"",""connection"":""ADBC_VS_MYSQL"",""database"":""adbc_vs"", ...omitted-columns...}"
```

`driverPath` is the versioned install path (`.../mysql/0.6.0/libadbc_driver_mysql.so`), and `driverVersion":"0.6.0"` is present — proving self-install fired and wrote the versioned path again.

```
$ exapump bucketfs ls -r adbc-drivers/mysql
0.6.0/libadbc_driver_mysql.so
supplied/libadbc_driver_mysql.so

$ docker exec labs-adbc-vs-4-exasol-1 ls -la /exa/data/bucketfs/bfsdefault/.dest/default/adbc-drivers/mysql/supplied/
-rwxr-xr-x 2 exadefusr exausers 46408368 Sep  3 18:22 libadbc_driver_mysql.so
```

Supplied object size `46408368` bytes matches the original round-tripped file exactly (`ls -la /tmp/.../libadbc_driver_mysql.so` → `46408368` bytes from row 4) — untouched by self-install.

Cleanup performed:

```
$ exapump sql --dsn "$DSN" 'DROP VIRTUAL SCHEMA REINSTALLVS CASCADE'
[1/1] DROP VIRTUAL SCHEMA REINSTALLVS CASCADE OK

$ exapump bucketfs rm adbc-drivers/mysql/supplied/libadbc_driver_mysql.so --bfs-write-password "<redacted>"
Deleted adbc-drivers/mysql/supplied/libadbc_driver_mysql.so

$ exapump bucketfs ls -r adbc-drivers/mysql
0.6.0/libadbc_driver_mysql.so
```

---

## Final clean-state sanity check

```
$ exapump sql --dsn "$DSN" "SELECT CONNECTION_NAME FROM SYS.EXA_ALL_CONNECTIONS WHERE CONNECTION_NAME='ADBC_VS_BUCKETFS'"
CONNECTION_NAME
ADBC_VS_BUCKETFS

$ exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA SANITYVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"
[1/1] CREATE VIRTUAL SCHEMA SANITYVS USING ADBC_VS.ADBC_ADAPTER... OK

$ exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM SANITYVS."orders"'
COUNT(*)
3

$ exapump sql --dsn "$DSN" 'DROP VIRTUAL SCHEMA SANITYVS CASCADE'
[1/1] DROP VIRTUAL SCHEMA SANITYVS CASCADE OK

$ docker exec labs-adbc-vs-4-exasol-1 iptables -L OUTPUT
Chain OUTPUT (policy ACCEPT)
target     prot opt source               destination        (empty)

$ exapump bucketfs ls -r adbc-drivers
postgresql/1.12.0/libadbc_driver_postgresql.so
mysql/0.6.0/libadbc_driver_mysql.so
```

Stack left in a clean, restored state: `ADBC_VS_BUCKETFS` connection present with its original definition, no `supplied` object left in BucketFS, no iptables rules, egress confirmed working, and a throwaway virtual schema with `BUCKETFS_CONNECTION` unset creates and drops cleanly with the correct row count — proving the default-connection path is usable for anyone picking up the stack next.

---

## Summary

| Row | Feature | Result |
|-----|---------|--------|
| 1 | default connection | PASS |
| 2 | explicit override never falls back | PASS |
| 3 | no route at all | PASS |
| 4 | pre-placed library, no connection | PASS |
| 5 | pre-placed library, unreachable registry | PASS |
| 6 | supplied file never written by self-install | PASS |

No code defects observed. No source code was modified as part of this verification.
