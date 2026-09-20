"""Distribution boundary: this checkout must stand on its own."""
import pathlib
import re
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class StandaloneContract(unittest.TestCase):
    def test_documentation_links_resolve_inside_this_checkout(self):
        for document in [ROOT / "README.md", *ROOT.glob("docs/**/*.md")]:
            for destination in re.findall(r"\]\(([^)]+)\)", document.read_text()):
                if "://" in destination or destination.startswith("#"):
                    continue
                path = (document.parent / destination.split("#", 1)[0]).resolve()
                self.assertTrue(path.is_relative_to(ROOT) and path.exists(),
                                f"{document.relative_to(ROOT)}: {destination}")

    def test_workspace_has_no_sy_or_external_path_dependencies(self):
        manifests = [ROOT / "Cargo.toml", *ROOT.glob("crates/*/Cargo.toml")]
        for manifest in manifests:
            data = tomllib.loads(manifest.read_text())
            self.assertNotIn(data.get("package", {}).get("name"), ("sy", "sy-core", "sy-ipc"))
            groups = [data, data.get("workspace", {})]
            for group in groups:
                for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                    for name, dependency in group.get(section, {}).items():
                        self.assertFalse(name.startswith("sy-"), name)
                        if isinstance(dependency, dict) and "path" in dependency:
                            path = (manifest.parent / dependency["path"]).resolve()
                            self.assertTrue(path.is_relative_to(ROOT), str(path))


if __name__ == "__main__":
    unittest.main()
