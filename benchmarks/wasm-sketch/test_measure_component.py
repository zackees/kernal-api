import unittest
import json
from pathlib import Path

from measure_component import edit_source, validate_output
from measure_core import edit_summary


class ComponentMeasurementTests(unittest.TestCase):
    def test_recorded_diagnostic_has_ten_distinct_compiled_edits(self):
        for profile in ("debug", "release"):
            with self.subTest(profile=profile):
                path = Path(__file__).parent / f"results/component-{profile}-diagnostic.json"
                record = json.loads(path.read_text(encoding="utf-8"))
                self.assertEqual(record["status"], "complete-diagnostic-only")
                self.assertEqual(record["encoder_profile"], profile)
                self.assertEqual(len(record["samples"]), 12)
                self.assertEqual(edit_summary(record["samples"][2:]), record["edit_summary"])
                for sample in record["samples"]:
                    self.assertGreater(sample["module_bytes"], 0)
                    self.assertGreater(sample["encode_and_compile_command_ns"], 0)

    def test_edit_changes_exactly_one_runtime_limit(self):
        source = "if total > 64 * 1024 * 1024 {"
        self.assertEqual(edit_source(source, 64, 65), "if total > 65 * 1024 * 1024 {")
        for invalid in ("no anchor", source * 2):
            with self.assertRaises(ValueError):
                edit_source(invalid, 64, 65)

    def test_requires_encoding_and_engine_compilation_of_exact_size(self):
        output = ("validated component: 123 bytes; one kernel import; not executed\n"
                  "Wasmtime 45 component compilation passed; not instantiated or executed\n")
        validate_output(output, 123)
        for invalid in ("", output.splitlines()[0], output.replace("123", "124"),
                        output + "unexpected output\n"):
            with self.assertRaises(ValueError):
                validate_output(invalid, 123)


if __name__ == "__main__":
    unittest.main()
