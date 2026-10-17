import itertools
import json
from pathlib import Path
import random
import re
import subprocess
import time
import unittest
import uuid


ROOT = Path(__file__).resolve().parents[1]
IMAGE = "timescale/timescaledb@sha256:2c718e700b1c75b93488085596f2254c991c3aa29584fc282ebbe62fd8335791"
MIGRATIONS = ROOT / "apps/api/migrations"
FIELDS = [
    "theme_palette", "theme_mode", "theme_selection_kind", "theme_builtin_id",
    "theme_custom_id", "theme_custom_revision", "theme_etag_version",
]


def literal(value):
    return "'" + json.dumps(value, ensure_ascii=True).replace("'", "''") + "'::jsonb"


def historical_function(file_name, name):
    source = (MIGRATIONS / file_name).read_text()
    match = re.search(
        r"CREATE OR REPLACE FUNCTION panda\." + re.escape(name) + r"\(.*?\$\$;",
        source, re.DOTALL,
    )
    if not match:
        raise AssertionError("Historical function missing: " + name)
    return match.group(0)


class StoredFunctionContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.container = "rtd-sql-contract-" + uuid.uuid4().hex[:12]
        command = [
            "docker", "run", "--detach", "--pull", "never", "--network", "none",
            "--name", cls.container, "--label", "rtd.test=stored-function-contract",
            "-e", "POSTGRES_PASSWORD=synthetic-fixture-password",
            "-e", "POSTGRES_DB=contracts", IMAGE,
        ]
        started = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True)
        if started.returncode:
            raise RuntimeError("Isolated PostgreSQL fixture did not start: " + started.stderr)
        try:
            cls.prepare_database()
        except BaseException:
            cls.cleanup()
            raise

    @classmethod
    def prepare_database(cls):
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            ready = subprocess.run(
                ["docker", "exec", cls.container, "pg_isready", "-h", "127.0.0.1", "-U", "postgres", "-d", "contracts"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            if ready.returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise RuntimeError("Isolated PostgreSQL fixture was not ready within 45 seconds")
        cls.sql("CREATE SCHEMA panda; CREATE EXTENSION pgcrypto;")
        cls.sql("""
            CREATE TABLE panda.theme_projection_contract(
              case_id integer PRIMARY KEY, theme_palette text, theme_mode text,
              theme_selection_kind text, theme_builtin_id text, theme_custom_id uuid,
              theme_custom_revision integer, theme_etag_version bigint
            );
        """)

    @classmethod
    def cleanup(cls):
        subprocess.run(
            ["docker", "rm", "--force", "--volumes", cls.container],
            check=True, stdout=subprocess.DEVNULL,
        )

    @classmethod
    def tearDownClass(cls):
        cls.cleanup()

    @classmethod
    def sql(cls, source):
        result = subprocess.run(
            ["docker", "exec", "-i", cls.container, "psql", "-X", "-qAt",
             "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1", "-U", "postgres", "-d", "contracts"],
            input=source, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            universal_newlines=True,
        )
        if result.returncode:
            raise AssertionError(result.stderr)
        return result.stdout.strip()

    def install_baseline(self):
        self.sql(historical_function("0007_charge_payload_identity.sql", "semantic_charge_payload"))
        self.sql((MIGRATIONS / "0010_charge_payload_identity_helpers.sql").read_text())
        self.sql(historical_function("0018_extensible_themes.sql", "project_legacy_theme_write"))
        self.sql("""
            DROP TRIGGER IF EXISTS project_contract ON panda.theme_projection_contract;
            CREATE TRIGGER project_contract BEFORE UPDATE ON panda.theme_projection_contract
            FOR EACH ROW EXECUTE FUNCTION panda.project_legacy_theme_write();
        """)

    def install_candidate(self):
        self.sql((MIGRATIONS / "0032_stored_function_implementations.sql").read_text())

    def test_semantic_json_and_persisted_fingerprints(self):
        fixtures = [
            None, True, False, 0, -7, 1.125, "", "updatedAt", [], {},
            {"updatedAt": 1, "UpdatedAt": 2, "updatedat": 3},
            {"a": [{"observedAt": 1, "energy": None}, {"energy": 0}, []]},
            {"'quoted": "snowman \u2603", "nested": {"retrievedAt": None}},
        ]
        randomizer = random.Random(197)
        keys = [
            "updatedAt", "retrievedAt", "capturedAt", "observedAt", "fetchedAt",
            "lastUpdatedAt", "startAt", "endAt", "energy", "power", "UpdatedAt",
        ]

        def value(depth):
            if depth == 0:
                return randomizer.choice([None, True, False, 0, -31, 2.5, "value", ""])
            if randomizer.randrange(2):
                return {key: value(depth - 1) for key in randomizer.sample(keys, randomizer.randrange(6))}
            return [value(depth - 1) for _ in range(randomizer.randrange(5))]

        fixtures.extend(value(4) for _ in range(160))
        rows = ",".join("(%d,%s)" % (index, literal(fixture)) for index, fixture in enumerate(fixtures))
        query = """
            WITH fixtures(id, value) AS (VALUES %s)
            SELECT jsonb_agg(jsonb_build_array(
              id, panda.semantic_charge_payload(value),
              encode(panda.charge_payload_fingerprint(value), 'hex'),
              encode(panda.charge_payload_identity_key(
                '00000000-0000-0000-0000-000000000007'::uuid, 'charge',
                NULL, 'synthetic-vehicle', panda.charge_payload_fingerprint(value)
              ), 'hex')
            ) ORDER BY id) FROM fixtures;
        """ % rows
        self.install_baseline()
        baseline = self.sql(query)
        self.install_candidate()
        candidate = self.sql(query)
        self.assertEqual(candidate, baseline)
        self.assertEqual(len(json.loads(candidate)), len(fixtures))
        self.assertEqual(self.sql("SELECT panda.semantic_charge_payload(NULL) IS NULL;"), "t")
        self.assertEqual(self.sql(
            "SELECT panda.semantic_charge_payload('{\"updatedAt\":1,\"energy\":2}')::text;"
        ), '{"energy": 2}')
        self.assertEqual(self.sql("""
            SELECT bool_and(p.provolatile = 'i' AND p.proisstrict AND NOT p.prosecdef)
            FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
            WHERE n.nspname='panda'
              AND p.proname IN ('semantic_charge_payload', 'charge_payload_fingerprint');
        """), "t")

    def test_legacy_theme_transition_matrix(self):
        original = {
            "theme_palette": "classic", "theme_mode": "dark",
            "theme_selection_kind": "custom", "theme_builtin_id": "classic",
            "theme_custom_id": "00000000-0000-0000-0000-000000000011",
            "theme_custom_revision": 2, "theme_etag_version": 9,
        }
        mutations = {
            "theme_selection_kind": "builtin", "theme_builtin_id": "rad",
            "theme_custom_id": "00000000-0000-0000-0000-000000000012",
            "theme_custom_revision": 3,
        }
        fixtures = []
        for null_base, palette, mode, etag, mask in itertools.product(
            [False, True], ["classic", "rad", None], ["dark", "light", None],
            [9, 18, None], range(16),
        ):
            before = {key: None if null_base else value for key, value in original.items()}
            after = dict(before, theme_palette=palette, theme_mode=mode, theme_etag_version=etag)
            for bit, (key, value) in enumerate(mutations.items()):
                if mask & (1 << bit):
                    after[key] = value
            case_id = len(fixtures)
            before["case_id"] = after["case_id"] = case_id
            fixtures.append((case_id, before, after))
        rows = ",".join("(%d,%s,%s)" % (index, literal(before), literal(after)) for index, before, after in fixtures)
        self.sql("CREATE TABLE panda.theme_transition_cases(id integer, before_value jsonb, after_value jsonb);"
                 "INSERT INTO panda.theme_transition_cases VALUES " + rows + ";")
        fields = ",".join(FIELDS)
        exercise = """
            TRUNCATE panda.theme_projection_contract;
            INSERT INTO panda.theme_projection_contract
            SELECT (jsonb_populate_record(NULL::panda.theme_projection_contract, before_value)).*
            FROM panda.theme_transition_cases;
            UPDATE panda.theme_projection_contract destination
            SET (%s) = (
              SELECT %s FROM jsonb_populate_record(NULL::panda.theme_projection_contract, cases.after_value)
            ) FROM panda.theme_transition_cases cases WHERE destination.case_id=cases.id;
            SELECT jsonb_agg(to_jsonb(t) ORDER BY case_id) FROM panda.theme_projection_contract t;
        """ % (fields, fields)
        self.install_baseline()
        baseline = self.sql(exercise)
        self.install_candidate()
        candidate = self.sql(exercise)
        self.assertEqual(candidate, baseline)
        self.assertEqual(len(json.loads(candidate)), 864)


if __name__ == "__main__":
    unittest.main()
