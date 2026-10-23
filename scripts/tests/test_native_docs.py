"""Exercise the real compiler frontend; never substitute a hand-written AST fixture."""

from pathlib import Path
import sys
import subprocess
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_native_docs import check_header


class NativeDocumentationTests(unittest.TestCase):
    def check(self, source, language="c", **options):
        with tempfile.TemporaryDirectory(prefix="rnet-docs-test-") as directory:
            header = Path(directory) / ("api.h" if language == "c" else "api.hpp")
            header.write_text(source, encoding="utf-8")
            return check_header(header, language, **options)

    def test_c_documents_are_attached_to_each_declaration(self):
        source = """
/** Limits for bounded admission. */
typedef struct rnet_limits {
  unsigned count; /**< Maximum queued items; positive. */
} rnet_limits_t;
enum { RNET_READY = 1 /**< Ready for business sends. */ };
/** Borrows bytes during the callback. */
typedef void (*rnet_callback_t)(const char *bytes);
/** Copies opaque bytes; returns zero on local admission. */
int rnet_send(const char *bytes);
"""
        errors, count = self.check(source)
        self.assertEqual(errors, [])
        self.assertGreaterEqual(count, 6)
        for comment, symbol in [
            ("/**< Maximum queued items; positive. */", "count"),
            ("/**< Ready for business sends. */", "RNET_READY"),
            ("/** Borrows bytes during the callback. */", "rnet_callback_t"),
            ("/** Copies opaque bytes; returns zero on local admission. */", "rnet_send"),
        ]:
            with self.subTest(symbol=symbol):
                errors, _ = self.check(source.replace(comment, ""))
                self.assertTrue(any(symbol in error for error in errors), errors)

    def test_type_documentation_does_not_cover_fields(self):
        errors, _ = self.check("/** Config. */ struct rnet_config { int limit; };")
        self.assertTrue(any("rnet_config::limit" in error for error in errors), errors)

    def test_generated_constant_macros_require_their_own_documentation(self):
        source = """
#ifndef RNET_H
#define RNET_H
/** Current SDK/native layout. */
#define RNET_ABI_VERSION 4
/** Local failure. */
#define RNET_FAILURE -1
#endif
"""
        errors, count = self.check(source)
        self.assertEqual(errors, [])
        self.assertEqual(count, 2)
        errors, _ = self.check(source.replace("/** Local failure. */", "/* internal note */"))
        self.assertTrue(any("RNET_FAILURE" in error for error in errors), errors)

    def test_empty_or_ordinary_comments_are_not_documentation(self):
        for prefix in ["/** */", "// internal note", "/* internal note */"]:
            with self.subTest(prefix=prefix):
                errors, _ = self.check(prefix + "\nint rnet_send(void);\n")
                self.assertTrue(any("rnet_send" in error for error in errors), errors)

    def test_cpp_access_and_function_bodies_are_not_confused(self):
        source = """
namespace rnet {
/// Owns one runtime.
class Runtime {
  int private_storage;
 public:
  /// Constructs a server runtime.
  Runtime() = default;
  /// Runtime has unique ownership.
  Runtime(const Runtime &) = delete;
  /// No-throw cleanup.
  ~Runtime() = default;
  /// Copies bytes before returning.
  void send() { struct Local { int not_public; }; }
 protected:
  /// Available to subclasses.
  int capacity;
 private:
  void helper();
};
/// Copied event metadata.
struct Event {
  int session; ///< Local session handle.
};
/// Selected security policy.
enum class Mode { Secure ///< Authenticated encryption.
};
}
"""
        errors, count = self.check(source, "c++")
        self.assertEqual(errors, [])
        self.assertGreaterEqual(count, 10)
        for comment, symbol in [
            ("/// Runtime has unique ownership.", "Runtime"),
            ("/// Available to subclasses.", "capacity"),
            ("///< Local session handle.", "session"),
            ("///< Authenticated encryption.", "Secure"),
        ]:
            with self.subTest(symbol=symbol):
                errors, _ = self.check(source.replace(comment, ""), "c++")
                self.assertTrue(any(symbol in error for error in errors), errors)

    def test_compiler_errors_and_missing_compiler_fail_closed(self):
        with self.assertRaises(RuntimeError):
            self.check("struct rnet_broken { !!! };")
        with self.assertRaises(RuntimeError):
            self.check("int rnet_send(void);", compiler="/nonexistent/rnet-clang")

    def test_invalid_documented_parameter_fails(self):
        with self.assertRaises(RuntimeError):
            self.check("/** @param unknown No such argument. */\nint rnet_send(int count);")

    def test_reopened_namespaces_do_not_lose_coverage(self):
        errors, count = self.check("""
namespace rnet { /** First operation. */ void first(); }
namespace rnet { void undocumented(); }
""", "c++")
        self.assertEqual(count, 2)
        self.assertEqual(len(errors), 1)
        self.assertIn("rnet::undocumented", errors[0])

    def test_cpp_dependencies_are_checked_separately(self):
        with tempfile.TemporaryDirectory(prefix="rnet-docs-include-") as directory:
            dependency = Path(directory) / "dependency.hpp"
            dependency.write_text("namespace rnet { struct Missing { int field; }; }", encoding="utf-8")
            header = Path(directory) / "api.hpp"
            header.write_text('#include "dependency.hpp"\nnamespace rnet {\n'
                              '/// Public operation.\nvoid operation();\n}', encoding="utf-8")
            errors, count = check_header(header, "c++")
            self.assertEqual(errors, [])
            self.assertEqual(count, 1)
            errors, count = check_header(dependency, "c++")
            self.assertEqual(len(errors), 2)
            self.assertEqual(count, 2)

    def test_c_dependency_diagnostics_point_to_the_owning_file(self):
        with tempfile.TemporaryDirectory(prefix="rnet-docs-include-") as directory:
            dependency = Path(directory) / "dependency.h"
            dependency.write_text("/** Public limits. */\nstruct rnet_limits { int count; };", encoding="utf-8")
            header = Path(directory) / "api.h"
            header.write_text('#include "dependency.h"\n/** Public call. */\nint rnet_send(void);', encoding="utf-8")
            errors, _ = check_header(header, "c")
            self.assertEqual(len(errors), 1)
            self.assertIn(f"{dependency}:2:", errors[0])

    def test_unsupported_public_templates_are_not_silently_ignored(self):
        with self.assertRaisesRegex(RuntimeError, "unsupported public declaration"):
            self.check("namespace rnet { template<class T> void call(T); }", "c++")

    def test_cli_checks_the_inventory_and_reports_nonzero_on_missing_docs(self):
        with tempfile.TemporaryDirectory(prefix="rnet-docs-cli-") as directory:
            root = Path(directory)
            (root / "include").mkdir()
            (root / "cpp/rnet").mkdir(parents=True)
            (root / "include/rnet.h").write_text("/** Copies data. */ int rnet_send(void);", encoding="utf-8")
            wrapper = root / "cpp/rnet/api.hpp"
            wrapper.write_text("namespace rnet { /** Copies data. */ void send(); }", encoding="utf-8")
            command = [sys.executable, "-B", str(Path(__file__).resolve().parents[1] / "check_native_docs.py"),
                       "--root", str(root)]
            def run():
                return subprocess.run(command, text=True, capture_output=True, timeout=30, check=False)
            valid = run()
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertIn("cpp/rnet/api.hpp: inspected 1 declarations", valid.stdout)
            wrapper.write_text("namespace rnet { void send(); }", encoding="utf-8")
            missing = run()
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("missing documentation for rnet::send", missing.stderr)
            wrapper.unlink()
            absent = run()
            self.assertNotEqual(absent.returncode, 0)
            self.assertIn("no C++ public headers", absent.stderr)


if __name__ == "__main__":
    unittest.main()
