"""Read-only GitHub provenance and approval checks for the current archive workflow.

Repository administrators and the default branch are trust roots. These checks do
not protect against an administrator changing repository policy or trusted code.
All API routes are constructed locally; response-provided URLs are never executed.
"""
import re
from urllib.parse import quote

from archive_validation import checked_identity

ENVIRONMENT = "rnet-release-archive"
WORKFLOW = ".github/workflows/release-archive.yml"
QUALIFICATION = ".github/workflows/release-qualification.yml"


def positive(value):
    if type(value) is not int or value <= 0:
        raise ValueError("expected positive GitHub numeric identity")
    return value


def prefix(repository):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid repository")
    return "/repos/" + repository


def execution(api, context):
    """Only first attempts on the trusted default branch can use review history.

    GitHub review records have no attempt identity. A rerun could reuse an earlier
    approval, so retries must be new manual dispatches with fresh environment approval.
    """
    p = prefix(context["repository"])
    if (context["attempt"] != "1" or not re.fullmatch(r"[1-9][0-9]{0,19}", context["run_id"])
            or not re.fullmatch(r"[0-9a-f]{40}", context["sha"])):
        raise ValueError("archive requires a fresh dispatch, not a workflow rerun")
    repo = api.get(p)
    rid = positive(repo["id"])
    branch = repo["default_branch"]
    run = api.get(p + "/actions/runs/" + context["run_id"])
    if (repo["full_name"] != context["repository"] or context["ref"] != "refs/heads/" + branch
            or run.get("id") != int(context["run_id"]) or run.get("run_attempt") != 1
            or run.get("repository", {}).get("id") != rid
            or run.get("head_repository", {}).get("id") != rid
            or run.get("head_branch") != branch or run.get("head_sha") != context["sha"]
            or run.get("event") != "workflow_dispatch" or run.get("path") != WORKFLOW
            or run.get("status") != "in_progress"):
        raise ValueError("untrusted archive execution context")
    return p, repo, run


def environment(api, p, branch):
    env = api.get(p + "/environments/" + ENVIRONMENT)
    rules = [r for r in env.get("protection_rules", []) if r.get("type") == "required_reviewers"]
    if (env.get("name") != ENVIRONMENT or len(rules) != 1
            or rules[0].get("prevent_self_review") is not True
            or env.get("deployment_branch_policy") != {
                "protected_branches": False, "custom_branch_policies": True}):
        raise ValueError("environment needs required reviewers, no self-review and exact branch policy")
    positive(env["id"])
    reviewers = rules[0].get("reviewers", [])
    # Direct users make authorization checkable without an organization-membership token.
    if not reviewers or any(r.get("type") != "User" for r in reviewers):
        raise ValueError("archive environment must name individual required reviewers")
    allowed = {positive(r["reviewer"]["id"]) for r in reviewers}
    policies = api.pages(p + "/environments/" + ENVIRONMENT + "/deployment-branch-policies", "branch_policies")
    if (len(policies) != 1 or policies[0].get("name") != branch
            or policies[0].get("type") != "branch"):
        raise ValueError("archive environment must allow only the exact default branch")
    return env, allowed


def require_approval(api, context):
    """Reject bypass, rejection, self-approval, foreign environments and stale reruns."""
    p, repo, run = execution(api, context)
    env, allowed = environment(api, p, repo["default_branch"])
    actors = {positive(run[name]["id"]) for name in ("actor", "triggering_actor")}
    reviews = api.get(p + "/actions/runs/" + context["run_id"] + "/approvals")
    if not isinstance(reviews, list):
        raise ValueError("invalid GitHub approval history")
    relevant = [r for r in reviews if any(e.get("id") == env["id"] for e in r.get("environments", []))]
    if any(r.get("state") != "approved" for r in relevant) or not any(
            r.get("user", {}).get("id") in allowed - actors for r in relevant):
        raise ValueError("no independent required reviewer approved this archive run")


def resolve_tag(api, p, tag):
    obj = api.get(p + "/git/ref/tags/" + quote(tag, safe=""))["object"]
    seen = set()
    for _ in range(8):
        sha = obj.get("sha", "")
        if not re.fullmatch(r"[0-9a-f]{40}", sha) or sha in seen:
            raise ValueError("invalid or cyclic tag reference")
        if obj.get("type") == "commit":
            return sha
        if obj.get("type") != "tag":
            break
        seen.add(sha)
        obj = api.get(p + "/git/tags/" + sha)["object"]
    raise ValueError("tag does not resolve to a bounded commit reference")


def require_tag_rules(api, p, tag):
    for item in api.pages(p + "/rulesets?includes_parents=true"):
        rule = api.get(p + "/rulesets/" + str(positive(item["id"])))
        refs = rule.get("conditions", {}).get("ref_name", {})
        if (rule.get("target") == "tag" and rule.get("enforcement") == "active"
                and refs.get("exclude") == []
                and any(v in refs.get("include", []) for v in ("~ALL", "refs/tags/" + tag))
                and {"update", "deletion"} <= {r.get("type") for r in rule.get("rules", [])}):
            return
    raise ValueError("tag needs an active exact-tag or all-tags update/deletion ruleset")


def collect_origin(api, identity, context):
    identity = checked_identity(identity)
    p, repo, _ = execution(api, context)
    if identity["repository"] != repo["full_name"] or identity["validator_revision"] != context["sha"]:
        raise ValueError("validator or repository identity mismatch")
    environment(api, p, repo["default_branch"])
    run_route = p + "/actions/runs/" + identity["run_id"]
    run = api.get(run_route)
    attempt = api.get(run_route + "/attempts/" + identity["attempt"])
    for record in (run, attempt):
        if (record.get("id") != int(identity["run_id"])
                or record.get("run_attempt") != int(identity["attempt"])
                or record.get("event") != "workflow_dispatch" or record.get("status") != "completed"
                or record.get("conclusion") != "success" or record.get("path") != QUALIFICATION
                or record.get("repository", {}).get("id") != repo["id"]
                or record.get("head_repository", {}).get("id") != repo["id"]
                or record.get("head_branch") != repo["default_branch"]
                or record.get("head_sha") != identity["commit"]):
            raise ValueError("qualification run/attempt/repository/source is not trusted or successful")
    workflow_id = positive(run["workflow_id"])
    workflow = api.get(p + "/actions/workflows/" + str(workflow_id))
    if (attempt.get("workflow_id") != workflow_id or workflow.get("id") != workflow_id
            or workflow.get("path") != QUALIFICATION or workflow.get("state") != "active"):
        raise ValueError("qualification workflow identity mismatch")
    comparison = api.get(p + "/compare/" + identity["commit"] + "..." + context["sha"])
    if comparison.get("status") not in ("ahead", "identical"):
        raise ValueError("candidate is not an ancestor of the trusted validator revision")
    if resolve_tag(api, p, identity["tag"]) != identity["commit"]:
        raise ValueError("release tag moved or targets a different commit")
    require_tag_rules(api, p, identity["tag"])
    artifacts = api.pages(run_route + "/artifacts", "artifacts")
    selected = {}
    for kind in ("evidence", "candidate"):
        name = f"{kind}-{identity['commit']}-{identity['run_id']}-{identity['attempt']}"
        matching = [a for a in artifacts if a.get("name") == name]
        if len(matching) != 1:
            raise ValueError("missing or ambiguous qualification artifact")
        a = matching[0]
        positive(a["id"])
        wr = a.get("workflow_run", {})
        if (a.get("expired") is not False or not isinstance(a.get("digest"), str)
                or not re.fullmatch(r"sha256:[0-9a-f]{64}", a["digest"])
                or type(a.get("size_in_bytes")) is not int or not 0 < a["size_in_bytes"] <= 1024**3
                or wr.get("id") != int(identity["run_id"]) or wr.get("repository_id") != repo["id"]
                or wr.get("head_repository_id") != repo["id"] or wr.get("head_sha") != identity["commit"]):
            raise ValueError("expired, foreign or unverifiable artifact")
        selected[kind] = {k: a[k] for k in ("id", "name", "digest", "size_in_bytes")}
    return {"schema": 1, "identity": identity, "repository_id": repo["id"],
            "qualification_workflow_id": workflow_id, "artifacts": selected,
            "qualification_url": f"https://github.com/{identity['repository']}/actions/runs/{identity['run_id']}/attempts/{identity['attempt']}"}
