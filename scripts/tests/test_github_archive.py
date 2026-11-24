"""Hosted origin and publication tests use synthetic evidence, never a real Release."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

from archive_fixture import fixture
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from evidence_source import sha256
from github_archive_origin import collect_origin, require_approval
from github_archive_publish import publish
from github_archive import prepare_hosted


class Hosting:
    def __init__(self, root):
        evidence, sdk, self.identity = fixture(root)
        i = self.identity
        self.repo = {"id": 42, "full_name": i["repository"], "default_branch": "main"}
        self.run = {"id": 101, "run_attempt": 2, "repository": self.repo,
                    "head_repository": self.repo, "head_sha": i["commit"], "head_branch": "main",
                    "event": "workflow_dispatch", "status": "completed", "conclusion": "success",
                    "path": ".github/workflows/release-qualification.yml", "workflow_id": 9}
        self.archive_run = {**self.run, "id": 201, "run_attempt": 1,
                            "head_sha": i["validator_revision"], "status": "in_progress",
                            "conclusion": None, "path": ".github/workflows/release-archive.yml",
                            "actor": {"id": 1}, "triggering_actor": {"id": 1}}
        self.context = {"repository": i["repository"], "run_id": "201", "attempt": "1",
                        "sha": i["validator_revision"], "ref": "refs/heads/main"}
        self.environment = {"id": 8, "name": "rnet-release-archive",
            "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True},
            "protection_rules": [{"type": "required_reviewers", "prevent_self_review": True,
                                  "reviewers": [{"type": "User", "reviewer": {"id": 2}}]}]}
        self.reviews = [{"state": "approved", "environments": [{"id": 8}], "user": {"id": 2}}]
        self.rules = {"id": 7, "target": "tag", "enforcement": "active",
                      "conditions": {"ref_name": {"include": ["refs/tags/candidate-001"], "exclude": []}},
                      "rules": [{"type": "update"}, {"type": "deletion"}]}
        self.artifacts, self.zips = [], {}
        for artifact_id, kind in enumerate(("evidence", "candidate"), 10):
            path = root / (kind + ".zip")
            with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
                files = evidence.rglob("*") if kind == "evidence" else [sdk, Path(str(sdk) + ".sha256")]
                for f in files:
                    if f.is_file(): z.write(f, f.relative_to(evidence) if kind == "evidence" else f.name)
            self.zips[artifact_id] = path
            self.artifacts.append({"id": artifact_id, "name": f"{kind}-{i['commit']}-101-2",
                "expired": False, "size_in_bytes": path.stat().st_size, "digest": "sha256:" + sha256(path),
                "workflow_run": {"id": 101, "repository_id": 42, "head_repository_id": 42,
                                 "head_sha": i["commit"]}})
        self.release = None
        self.assets = []
        self.writes = []
        self.fail_upload = False
        self.tag = i["commit"]

    def get(self, route):
        suffix = route.removeprefix("/repos/example/netlib")
        if suffix == "": return copy.deepcopy(self.repo)
        if suffix == "/actions/runs/101": return copy.deepcopy(self.run)
        if suffix == "/actions/runs/101/attempts/2": return copy.deepcopy(self.run)
        if suffix == "/actions/runs/" + self.context["run_id"]: return copy.deepcopy(self.archive_run)
        if suffix == "/actions/runs/" + self.context["run_id"] + "/approvals": return copy.deepcopy(self.reviews)
        if suffix == "/actions/workflows/9": return {"id": 9, "path": self.run["path"], "state": "active"}
        if suffix.startswith("/git/ref/tags/"): return {"object": {"type": "commit", "sha": self.tag}}
        if suffix.startswith("/compare/"): return {"status": "ahead"}
        if suffix == "/environments/rnet-release-archive": return copy.deepcopy(self.environment)
        if suffix == "/rulesets/7": return copy.deepcopy(self.rules)
        if suffix == "/releases/77": return copy.deepcopy(self.release)
        raise AssertionError(route)

    def pages(self, route, key=None):
        if "/artifacts" in route: return copy.deepcopy(self.artifacts)
        if "/deployment-branch-policies" in route: return [{"name": "main", "type": "branch"}]
        if "/rulesets" in route: return [{"id": 7}]
        if route.endswith("/releases"): return [copy.deepcopy(self.release)] if self.release else []
        if route.endswith("/assets"): return copy.deepcopy(self.assets)
        raise AssertionError(route)

    def download(self, route, output, maximum):
        artifact_id = int(route.split("/")[-2])
        output.write_bytes(self.zips[artifact_id].read_bytes())

    def write(self, method, route, body):
        self.writes.append((method, route, copy.deepcopy(body)))
        if method == "POST":
            self.release = {"id": 77, "author": {"login": "github-actions[bot]"}, **body}
        else:
            self.release.update(body)
        return copy.deepcopy(self.release)

    def upload(self, repository, release_id, path):
        self.writes.append(("UPLOAD", path.name))
        if self.fail_upload: raise ValueError("synthetic upload failure")
        self.assets.append({"id": len(self.assets) + 1, "name": path.name, "state": "uploaded",
                            "size": path.stat().st_size, "digest": "sha256:" + sha256(path)})


class GitHubArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.api = Hosting(self.root)

    def origin(self):
        return collect_origin(self.api, self.api.identity, self.api.context)

    def prepared(self):
        output = self.root / "prepared"
        return prepare_hosted(self.api, self.api.identity, self.api.context, output)

    def test_actual_zip_and_offline_validation_precede_any_write(self):
        prepared = self.prepared()
        self.assertEqual(self.api.writes, [])
        self.assertEqual(len(prepared["files"]), 6)
        publish(self.api, prepared, self.api.context, prepared["binding"])
        self.assertTrue(self.api.release["prerelease"])
        self.assertFalse(self.api.release["draft"])
        self.assertEqual(self.api.release["make_latest"], "false")
        self.assertEqual(len(self.api.assets), 6)

    def test_foreign_failed_rerun_or_wrong_workflow_is_rejected(self):
        for field, value in (("repository", {"id": 99}), ("head_repository", {"id": 99}),
                             ("head_sha", "c" * 40), ("run_attempt", 3),
                             ("head_branch", "untrusted"), ("event", "pull_request"),
                             ("conclusion", "failure"), ("path", "other.yml")):
            with self.subTest(field=field):
                old = self.api.run[field]; self.api.run[field] = value
                with self.assertRaises(ValueError): self.origin()
                self.api.run[field] = old
        self.assertEqual(self.api.writes, [])

    def test_expired_mixed_ambiguous_or_tampered_artifacts_fail(self):
        original = copy.deepcopy(self.api.artifacts)
        for change in (lambda a: a[0].update(expired=True),
                       lambda a: a[0].update(digest=None),
                       lambda a: a[0]["workflow_run"].update(head_repository_id=99),
                       lambda a: a.append(copy.deepcopy(a[0]))):
            change(self.api.artifacts)
            with self.assertRaises(ValueError): self.origin()
            self.api.artifacts = copy.deepcopy(original)
        self.api.zips[10].write_bytes(b"changed zip")
        with self.assertRaises(ValueError): self.prepared()
        self.assertEqual(self.api.writes, [])

    def test_missing_protection_tag_drift_and_approval_fail_closed(self):
        for reviews in ([], [{"state": "rejected", "environments": [{"id": 8}], "user": {"id": 2}}],
                        [{"state": "approved", "environments": [{"id": 8}], "user": {"id": 1}}],
                        [{"state": "approved", "environments": [{"id": 8}], "user": {"id": 99}}]):
            self.api.reviews = reviews
            with self.assertRaises(ValueError): require_approval(self.api, self.api.context)
        self.api.environment["protection_rules"][0]["prevent_self_review"] = False
        with self.assertRaises(ValueError): self.origin()
        self.api.environment["protection_rules"][0]["prevent_self_review"] = True
        self.api.tag = "c" * 40
        with self.assertRaises(ValueError): self.origin()
        self.api.tag = self.api.identity["commit"]
        self.api.rules["enforcement"] = "evaluate"
        with self.assertRaises(ValueError): self.origin()
        self.assertEqual(self.api.writes, [])

    def test_publish_rechecks_approval_binding_and_tag_before_writes(self):
        p = self.prepared()
        self.api.reviews = []
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(self.api.writes, [])
        self.api.reviews = [{"state": "approved", "environments": [{"id": 8}], "user": {"id": 2}}]
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, "f" * 64)
        self.api.tag = "c" * 40
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(self.api.writes, [])

    def test_failed_upload_leaves_draft_and_identical_retry_finishes(self):
        p = self.prepared(); self.api.fail_upload = True
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertTrue(self.api.release["draft"])
        self.api.fail_upload = False
        publish(self.api, p, self.api.context, p["binding"])
        before = list(self.api.writes)
        publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(self.api.writes, before)

    def test_conflicting_asset_is_never_deleted_replaced_or_published(self):
        p = self.prepared(); self.api.fail_upload = True
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.api.fail_upload = False
        self.api.assets = [{"id": 1, "name": next(iter(p["files"])), "state": "uploaded",
                            "size": 1, "digest": "sha256:" + "f" * 64}]
        before = list(self.api.writes)
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(before, self.api.writes)
        self.assertTrue(self.api.release["draft"])

    def test_rerun_cannot_reuse_old_approval(self):
        self.api.context["attempt"] = "2"
        self.api.archive_run["run_attempt"] = 2
        with self.assertRaises(ValueError): self.origin()
        with self.assertRaises(ValueError): require_approval(self.api, self.api.context)

    def test_remote_release_metadata_drift_leaves_draft(self):
        p = self.prepared()
        get = self.api.get
        def changed(route):
            value = get(route)
            if route.endswith("/releases/77"): value["name"] = "unexpected release"
            return value
        self.api.get = changed
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertTrue(self.api.release["draft"])
        self.assertFalse(any(w[0] == "PATCH" for w in self.api.writes))

    def test_fresh_dispatch_can_resume_identical_draft_but_needs_fresh_approval(self):
        p = self.prepared(); self.api.fail_upload = True
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.api.fail_upload = False
        self.api.context["run_id"] = "202"; self.api.archive_run["id"] = 202
        self.api.reviews = []
        before = list(self.api.writes)
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(before, self.api.writes)
        self.api.reviews = [{"state": "approved", "environments": [{"id": 8}], "user": {"id": 2}}]
        publish(self.api, p, self.api.context, p["binding"])
        self.assertFalse(self.api.release["draft"])
        self.assertIn("Published archive workflow: https://github.com/example/netlib/actions/runs/202", self.api.release["body"])

    def test_partial_starter_asset_is_never_automatically_deleted(self):
        p = self.prepared(); self.api.fail_upload = True
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.api.fail_upload = False
        self.api.assets = [{"name": next(iter(p["files"])), "state": "starter", "size": 0, "digest": None}]
        before = list(self.api.writes)
        with self.assertRaises(ValueError): publish(self.api, p, self.api.context, p["binding"])
        self.assertEqual(before, self.api.writes)
