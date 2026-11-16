#!/usr/bin/env python3
"""Real isolated PostgreSQL bootstrap tests. Never reads deployment credentials."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "database.py"
spec = importlib.util.spec_from_file_location("cloud_database", SCRIPT)
database = importlib.util.module_from_spec(spec)
spec.loader.exec_module(database)


class DatabaseBootstrapTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not all(shutil.which(binary) for binary in ("initdb", "pg_ctl", "psql")):
            raise unittest.SkipTest("PostgreSQL client/server tools required")
        cls.temporary = tempfile.TemporaryDirectory(prefix="zuse-schema-test-")
        cls.root = Path(cls.temporary.name)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            cls.port = sock.getsockname()[1]
        cls.clean_env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
        subprocess.run(["initdb", "-D", str(cls.root / "data"), "--auth=trust", "--username=zuse_test", "--no-locale"],
                       env=cls.clean_env, check=True, capture_output=True)
        subprocess.run(["pg_ctl", "-D", str(cls.root / "data"), "-l", str(cls.root / "server.log"),
                        "-o", f"-h 127.0.0.1 -p {cls.port} -k {cls.root}", "-w", "start"],
                       env=cls.clean_env, check=True, capture_output=True)
        cls.admin = f"postgresql://zuse_test@127.0.0.1:{cls.port}/postgres?sslmode=disable"

    @classmethod
    def tearDownClass(cls):
        try:
            subprocess.run(["pg_ctl", "-D", str(cls.root / "data"), "-m", "fast", "-w", "stop"],
                           env=cls.clean_env, check=True, capture_output=True)
        finally:
            cls.temporary.cleanup()

    def setUp(self):
        import uuid
        name = "bootstrap_" + uuid.uuid4().hex
        database.execute(self.admin, f'CREATE DATABASE "{name}";')
        self.url = f"postgresql://zuse_test@127.0.0.1:{self.port}/{name}?sslmode=disable"

    def test_release_manifest_contains_command_and_recreation_after_admission(self):
        expected=("0034_runtime_releases.sql","0035_launch_asset_release.sql","0036_harness_bundles.sql","0037_user_images.sql","0038_app_role_grants.sql","0041_user_image_repositories.sql","0042_machine_versions_grants.sql","0043_work_wakeups.sql")
        self.assertEqual(database.FILES[-8:],expected)
        self.assertEqual(len(database.FILES),39)
        self.assertIn("0026_task_command_admission.sql",database.FILES)
        self.assertNotIn("0022_computer_recreations.sql",database.FILES)
        self.assertNotIn("0022_recreation.sql",database.FILES)

    def test_work_triggers_notify_only_their_schema_after_commit(self):
        database.execute(self.url, database.render("production"))
        database.execute(self.url, database.render("staging"))
        sql = """SET search_path TO zuse_production;
LISTEN "zuse_cloud_work:zuse_production";
INSERT INTO zuse_rust_jobs(job_id,idempotency_key,kind,resource_id,resource_generation,payload,due_at,created_at,updated_at) VALUES('j1','k1','lifecycle','w1',1,'{}',1,1,1);
UPDATE zuse_rust_jobs SET state='running',lease_token='t',lease_expires_at=5 WHERE job_id='j1';
SELECT 'after-claim';
UPDATE zuse_rust_jobs SET state='completed',lease_token=NULL,lease_expires_at=NULL WHERE job_id='j1';
SELECT 'after-complete';
INSERT INTO zuse_staging.zuse_rust_jobs(job_id,idempotency_key,kind,resource_id,resource_generation,payload,due_at,created_at,updated_at) VALUES('j2','k2','lifecycle','w1',1,'{}',1,1,1);
SELECT 'after-other-schema';
BEGIN;
INSERT INTO zuse_rust_account_event_outbox(owner_id,kind,resource_id,created_at) VALUES('o','workspace','w1',1),('o','workspace','w2',1);
SELECT 'inside-transaction';
COMMIT;
"""
        result = subprocess.run(["psql", "-X", "--no-align", "--tuples-only", "-v", "ON_ERROR_STOP=1"], input=sql,
                                env=database.connection_environment(self.url), capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = [line for line in result.stdout.splitlines() if line.strip()]
        notices = [line for line in lines if line.startswith("Asynchronous notification")]
        self.assertEqual(len(notices), 2, result.stdout)
        self.assertIn('payload "jobs"', notices[0])
        self.assertIn('payload "account_events"', notices[1])
        # Claims, completions and the other schema stay silent; the outbox
        # notification arrives once, after commit.
        self.assertLess(lines.index(notices[0]), lines.index("after-claim"))
        self.assertLess(lines.index("inside-transaction"), lines.index(notices[1]))

    def test_fresh_staging_and_production_are_empty_isolated_and_idempotent(self):
        for environment in ("staging", "production"):
            first = database.execute(self.url, database.render(environment))[-1]
            second = database.execute(self.url, database.render(environment))[-1]
            self.assertEqual(first, second)
            self.assertEqual(first["migrationCount"], len(database.FILES))
            self.assertGreater(first["applicationTableCount"], 40)
        result = database.execute(self.url, """SELECT json_build_object(
            'staging', (SELECT count(*) FROM zuse_staging.api_cloud_workspaces),
            'production', (SELECT count(*) FROM zuse_production.api_cloud_workspaces),
            'publicTables', (SELECT count(*) FROM pg_tables WHERE schemaname='public'));
        """)[-1]
        self.assertEqual(result, {"staging": 0, "production": 0, "publicTables": 0})
        env = database.connection_environment(self.url, self.clean_env)
        env["ZUSE_CLOUD_DATABASE_URL"] = self.url
        checked = subprocess.run(["python3", str(SCRIPT), "--environment", "staging", "check"],
                                 env=env, check=True, text=True, capture_output=True)
        self.assertEqual(json.loads(checked.stdout)["environment"], "staging")

    def test_application_roles_are_isolated_from_other_environment_and_ledgers(self):
        provision_spec = importlib.util.spec_from_file_location("cloud_provision", SCRIPT.parent / "single-host/provision.py")
        provision = importlib.util.module_from_spec(provision_spec)
        provision_spec.loader.exec_module(provision)
        for environment in ("staging", "production"):
            database.execute(self.url, database.render(environment))
            name = self.url.split("/")[-1].split("?")[0]
            database.execute(self.url, provision.role_sql(environment, name, "synthetic-test-password", True))
        for environment in ("staging", "production"):
            role = f"zuse_cloud_{environment}"
            other = "zuse_production" if environment == "staging" else "zuse_staging"
            url = self.url.replace("zuse_test@", role + ":synthetic-test-password@")
            result = database.execute(url, f"SELECT json_build_object('schema',current_schema(),'otherUsage',has_schema_privilege(current_user,'{other}','USAGE'),'ledgerWrite',has_table_privilege(current_user,'zuse_cloud_schema_migrations','UPDATE'),'workspaceWrite',has_table_privilege(current_user,'api_cloud_workspaces','INSERT'),'baseWrite',has_table_privilege(current_user,'cloud_managed_bases','INSERT,UPDATE,DELETE'),'baseLockExecutable',has_function_privilege(current_user,current_schema()||'.zuse_rust_managed_base_admission_lock(text,text)','EXECUTE')); ")[-1]
            self.assertEqual(result, {"schema": f"zuse_{environment}", "otherUsage": False, "ledgerWrite": False, "workspaceWrite": True, "baseWrite": False, "baseLockExecutable": True})
        key = json.loads(provision.signing_key())
        self.assertEqual(key["crv"], "Ed25519")
        self.assertEqual(len(key["d"]), 43)

    def test_checksum_drift_is_rejected(self):
        database.execute(self.url, database.render("staging"))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in database.FILES:
                shutil.copyfile(database.MIGRATIONS / name, root / name)
            with (root / database.FILES[0]).open("a") as target:
                target.write("\n-- drift\n")
            with self.assertRaisesRegex(ValueError, "zuse_schema_checksum_mismatch"):
                database.execute(self.url, database.render("staging", root))

    def test_brokeboy_optional_install_is_explicit_idempotent_and_checked(self):
        database.execute(self.url,database.render("staging"))
        role_spec=importlib.util.spec_from_file_location("billing_provision",SCRIPT.parent / "single-host/provision.py")
        provision=importlib.util.module_from_spec(role_spec)
        role_spec.loader.exec_module(provision)
        name=self.url.split("/")[-1].split("?")[0]
        role=database.execute(self.url,"SELECT json_build_object('present',EXISTS(SELECT 1 FROM pg_roles WHERE rolname='zuse_cloud_staging'));")[-1]
        database.execute(self.url,provision.role_sql("staging",name,"synthetic-test-password",not role["present"]))
        before=database.execute(self.url,"SELECT json_build_object('present',to_regclass('zuse_staging.brokeboy_customers') IS NOT NULL);")[-1]
        self.assertFalse(before["present"])
        for _ in range(2):
            database.execute(self.url,database.render_brokeboy("staging"))
        result=database.execute(self.url,"SET search_path=zuse_staging,pg_catalog; SELECT json_build_object('count',(SELECT count(*) FROM zuse_cloud_schema_migrations WHERE name='0032_brokeboy.sql'),'migrations',(SELECT json_agg(json_build_object('name',name,'sha256',sha256)) FROM zuse_cloud_schema_migrations));")[-1]
        self.assertEqual(result["count"],1)
        self.assertIn("0033_shared_credits.sql", [m["name"] for m in result["migrations"]])
        self.assertTrue(database.manifest_matches("staging",result["migrations"]))
        grants=database.execute(self.url,"SELECT json_build_object('billing',has_table_privilege('zuse_cloud_staging','zuse_staging.brokeboy_customers','INSERT,UPDATE,DELETE'),'compute',has_table_privilege('zuse_cloud_staging','zuse_staging.zuse_brokeboy_compute_charges','INSERT,UPDATE,DELETE'),'requests',has_table_privilege('zuse_cloud_staging','zuse_staging.zuse_brokeboy_model_requests','INSERT,UPDATE,DELETE'),'ledger',has_table_privilege('zuse_cloud_staging','zuse_staging.zuse_cloud_schema_migrations','UPDATE'));")[-1]
        self.assertEqual(grants,{"billing":True,"compute":True,"requests":True,"ledger":False})
        database.execute(self.url,"UPDATE zuse_staging.zuse_cloud_schema_migrations SET sha256=repeat('0',64) WHERE name='0032_brokeboy.sql';")
        with self.assertRaisesRegex(ValueError,"zuse_schema_manifest_mismatch"):
            database.execute(self.url,database.render_brokeboy("staging"))

    def test_failed_migration_rolls_back_entire_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in database.FILES:
                shutil.copyfile(database.MIGRATIONS / name, root / name)
            with (root / database.FILES[-1]).open("a") as target:
                target.write("\nSELECT 1/0;\n")
            with self.assertRaisesRegex(ValueError, "zuse_schema_database_operation_failed"):
                database.execute(self.url, database.render("staging", root))
        result = database.execute(self.url, "SELECT json_build_object('present',EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='zuse_staging'));")[-1]
        self.assertFalse(result["present"])

    def test_existing_unmanaged_schema_is_not_adopted(self):
        database.execute(self.url, "CREATE SCHEMA zuse_staging; CREATE TABLE zuse_staging.existing(body text); INSERT INTO zuse_staging.existing VALUES('preserve');")
        with self.assertRaisesRegex(ValueError, "zuse_schema_existing_tables_require_import_review"):
            database.execute(self.url, database.render("staging"))
        result = database.execute(self.url, "SELECT json_build_object('body',body) FROM zuse_staging.existing;")[-1]
        self.assertEqual(result["body"], "preserve")

    def test_remote_tls_required_and_inherited_pg_settings_are_removed(self):
        with self.assertRaisesRegex(ValueError, "zuse_schema_verified_tls_required"):
            database.connection_environment("postgres://user:secret@remote.example/db?sslmode=require")
        env = database.connection_environment("postgres://user:s%40fe@remote.example/db?sslmode=verify-full", {
            "PGSERVICE": "wrong", "PGOPTIONS": "-c search_path=public", "DATABASE_URL": "unrelated", "PATH": "/bin"})
        self.assertEqual(env["PGPASSWORD"], "s@fe")
        self.assertNotIn("PGSERVICE", env)
        self.assertNotIn("PGOPTIONS", env)
        self.assertNotIn("DATABASE_URL", env)


if __name__ == "__main__":
    unittest.main()
