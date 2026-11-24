"""Publish already verified data as a candidate Release after fresh hosting checks.

No tag creation, deletion or attachment replacement is supported. Interrupted
uploads leave a draft. Resume with a fresh approved dispatch and identical inputs.
"""
import hashlib
import json
import re

from archive_io import regular_file
from evidence_source import sha256
from github_archive_origin import collect_origin, prefix, require_approval, resolve_tag
from release_archive import verify_archive


def file_binding(files):
    records = {name: {"sha256": sha256(regular_file(path)), "size": path.stat().st_size}
               for name, path in sorted(files.items())}
    binding = hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return binding, records


def _assets(api, p, release_id, records, *, complete):
    assets = api.pages(p + f"/releases/{release_id}/assets")
    names = set()
    for a in assets:
        name = a.get("name")
        if name in names or name not in records:
            raise ValueError("unexpected or duplicate release asset")
        names.add(name)
        record = records[name]
        if (a.get("state") != "uploaded" or a.get("size") != record["size"]
                or a.get("digest") != "sha256:" + record["sha256"]):
            raise ValueError("release asset conflicts with approved bytes; refusing overwrite")
    if complete and names != records.keys():
        raise ValueError("release attachments incomplete")
    return names


def publish(api, prepared, context, expected_binding):
    """Revalidate immediately before writes, then seal only a complete draft.

    expected_binding comes from the read-only job, not downloaded manifest claims.
    Hash matching is not a guarantee against privileged concurrent repository writers.
    """
    identity, files = prepared["origin"]["identity"], prepared["files"]
    require_approval(api, context)
    if collect_origin(api, identity, context) != prepared["origin"]:
        raise ValueError("hosting origin changed since verification")
    verify_archive(prepared["directory"], identity)
    binding, records = file_binding(files)
    if binding != expected_binding or binding != prepared["binding"]:
        raise ValueError("archive bytes differ from the approved verification job")
    p = prefix(identity["repository"])
    body_prefix = (f"RNet candidate archive (not production certified).\n\n"
                   f"Attachment binding SHA256: {binding}\n"
                   f"Qualified source: {identity['commit']}\n"
                   f"Qualification: {prepared['origin']['qualification_url']}\n"
                   f"Validator: {identity['validator_revision']}\n"
                   "Target weak-network/long-soak and independent security review remain outstanding.\n"
                   "Initial archive workflow: ")
    workflow_url = f"https://github.com/{identity['repository']}/actions/runs/"
    name = "RNet candidate " + identity["tag"]
    matches = [r for r in api.pages(p + "/releases") if r.get("tag_name") == identity["tag"]]
    if len(matches) > 1:
        raise ValueError("ambiguous existing releases")
    if matches:
        release = matches[0]
        suffix = release.get("body", "").removeprefix(body_prefix)
        run_link = re.escape(workflow_url) + r"[1-9][0-9]{0,19}\n"
        expected_suffix = run_link if release.get("draft") is True else run_link + "Published archive workflow: " + run_link
        if (release.get("name") != name or release.get("prerelease") is not True
                or release.get("author", {}).get("login") != "github-actions[bot]"
                or not release.get("body", "").startswith(body_prefix)
                or type(release.get("draft")) is not bool
                or not re.fullmatch(expected_suffix, suffix)):
            raise ValueError("existing release has a conflicting identity")
    else:
        release = api.write("POST", p + "/releases", {
            "tag_name": identity["tag"], "target_commitish": identity["commit"], "name": name,
            "body": body_prefix + workflow_url + context["run_id"] + "\n",
            "draft": True, "prerelease": True, "make_latest": "false"})
    release_id = release["id"]
    present = _assets(api, p, release_id, records, complete=not release["draft"])
    if not release["draft"]:
        return release
    for filename in sorted(records):
        if filename not in present:
            # Never retry an ambiguous write automatically. A later dispatch rereads state.
            api.upload(identity["repository"], release_id, files[filename])
    _assets(api, p, release_id, records, complete=True)
    require_approval(api, context)
    if collect_origin(api, identity, context) != prepared["origin"]:
        raise ValueError("origin changed while uploading; leaving draft")
    current = api.get(p + f"/releases/{release_id}")
    if (any(current.get(key) != release.get(key) for key in ("body", "tag_name", "name", "prerelease"))
            or current.get("author", {}).get("login") != "github-actions[bot]"):
        raise ValueError("release metadata changed during upload")
    if resolve_tag(api, p, identity["tag"]) != identity["commit"]:
        raise ValueError("tag moved during upload")
    # Recheck all assets after the remote metadata reads before making anything public.
    _assets(api, p, release_id, records, complete=True)
    if current.get("draft") is not True:
        raise ValueError("another publisher changed draft state")
    return api.write("PATCH", p + f"/releases/{release_id}",
                     {"draft": False, "prerelease": True, "make_latest": "false",
                      "body": current["body"] + "Published archive workflow: " + workflow_url + context["run_id"] + "\n"})
