"""Check documentation attached to public C/C++ declarations using Clang's AST.

C scope is rnet_ declarations and RNET_ enumerators; C++ scope is namespace rnet.
Macros and namespace summaries are outside this presence gate. Compiler diagnostics
validate documentation syntax, not whether prose correctly describes runtime behavior.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


DECLARATIONS = {
    "RecordDecl", "CXXRecordDecl", "EnumDecl", "EnumConstantDecl", "FieldDecl",
    "FunctionDecl", "CXXMethodDecl", "CXXConstructorDecl", "CXXDestructorDecl",
    "CXXConversionDecl", "TypedefDecl", "TypeAliasDecl", "VarDecl",
}
RECORDS = {"RecordDecl", "CXXRecordDecl"}


def ast_objects(text):
    """Clang's filtered dump is a stream of JSON objects, not one JSON array."""
    decoder = json.JSONDecoder()
    position = 0
    while position < len(text):
        if text[position].isspace():
            position += 1
            continue
        try:
            node, position = decoder.raw_decode(text, position)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"invalid Clang AST JSON: {error}") from error
        if not isinstance(node, dict) or not isinstance(node.get("kind"), str):
            raise RuntimeError("Clang AST object has no declaration kind")
        yield node


def has_documentation(node):
    def has_text(comment):
        return bool(comment.get("text", "").strip()) or any(
            has_text(child) for child in comment.get("inner", [])
        )
    return any(child.get("kind") == "FullComment" and has_text(child)
               for child in node.get("inner", []))


def check_header(header: Path, language: str, include_dirs=(), compiler=None):
    """Return missing-document diagnostics and the number of inspected declarations."""
    header = header.resolve()
    if language not in ("c", "c++"):
        raise ValueError("language must be c or c++")
    compiler = compiler or os.environ.get(
        "CLANG" if language == "c" else "CLANGXX",
        "clang" if language == "c" else "clang++",
    )
    command = [compiler, "-x", language, "-std=c11" if language == "c" else "-std=c++11",
               "-fsyntax-only", "-Wdocumentation", "-Werror", "-Xclang", "-ast-dump=json"]
    if language == "c++":
        # Avoid retaining the standard library's AST. RNet namespace fragments still include
        # each complete class, including access markers needed to identify the public surface.
        command += ["-Xclang", "-ast-dump-filter", "-Xclang", "rnet"]
    command += [f"-I{Path(path).resolve()}" for path in include_dirs] + [str(header)]
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=60, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"cannot run {compiler} for {header}: {error}") from error
    if result.returncode:
        raise RuntimeError(f"{header}: compiler exited {result.returncode}\n{result.stderr}")
    if not result.stdout.strip():
        raise RuntimeError(f"{header}: compiler emitted no scoped AST")
    sources = {header: header.read_bytes()}
    current_file = header
    missing = []
    count = 0

    def visit(node, scope=(), owned=False):
        nonlocal count, current_file
        loc = node.get("loc", {})
        # Clang elides repeated file names. Carry source ownership through declaration order,
        # including uninspected dependency declarations, instead of blaming the entry header.
        if loc.get("file"):
            current_file = Path(loc["file"]).resolve()
        kind = node.get("kind", "")
        name = node.get("name", "")
        children = node.get("inner", [])
        if node.get("isImplicit"):
            return
        if kind in ("TranslationUnitDecl", "LinkageSpecDecl"):
            for child in children:
                visit(child, scope, owned)
            return
        if kind == "NamespaceDecl":
            if name and (name == "rnet" or owned):
                for child in children:
                    visit(child, scope + (name,), True)
            return
        if kind == "EnumDecl" and not name:
            for child in children:
                visit(child, scope, owned)
            return
        if not owned and not name.startswith(("rnet_", "RNET_")):
            return
        qualified = "::".join(scope + (name,))
        if kind in DECLARATIONS:
            count += 1
            if not has_documentation(node):
                line = loc.get("line")
                if not line:
                    if current_file not in sources:
                        sources[current_file] = current_file.read_bytes()
                    line = sources[current_file][:loc.get("offset", 0)].count(b"\n") + 1
                missing.append(f"{current_file}:{line}: missing documentation for {qualified} ({kind})")
        elif kind.endswith("Decl") and kind not in ("StaticAssertDecl", "EmptyDecl"):
            raise RuntimeError(f"{header}: unsupported public declaration {kind}: {qualified}")
        if kind in RECORDS:
            access = "private" if node.get("tagUsed") == "class" else "public"
            for child in children:
                if child.get("kind") == "AccessSpecDecl":
                    access = child["access"]
                elif access != "private":
                    visit(child, scope + (name,), True)
        elif kind == "EnumDecl":
            for child in children:
                visit(child, scope + (name,), True)
        # Do not descend into method bodies, initializers or type expressions: local types,
        # parameters and implicit instantiations are not new exported declarations.

    for node in ast_objects(result.stdout):
        if language == "c++":
            if node.get("kind") != "NamespaceDecl":
                continue
            # Each public header is compiled independently. Dependencies are checked on their
            # own invocation, avoiding duplicate counts and misattributed source positions.
            file = node.get("loc", {}).get("file")
            if not file or Path(file).resolve() != header:
                continue
        visit(node)
    return missing, count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    root = args.root.resolve()
    headers = [(root / "include/rnet.h", "c")]
    headers += [(path, "c++") for path in sorted((root / "cpp/rnet").glob("*.hpp"))]
    if len(headers) == 1:
        raise RuntimeError("no C++ public headers found")
    errors = []
    for header, language in headers:
        missing, count = check_header(header, language, (root / "include", root / "cpp"))
        if not count:
            raise RuntimeError(f"{header}: no public declarations inspected")
        print(f"{header.relative_to(root)}: inspected {count} declarations, {len(missing)} missing")
        errors.extend(missing)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as error:
        print(error, file=sys.stderr)
        sys.exit(1)
