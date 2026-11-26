"""Build the browser renderer and the hosted phone workflow for the Papamo site."""
import argparse
import pathlib
import plistlib
import subprocess

here = pathlib.Path(__file__).resolve().parent
repo = here.parents[1]
args = argparse.ArgumentParser()
args.add_argument("output", type=pathlib.Path, help="The site's public/psms-inbox directory")
args.add_argument("--wasm-bindgen", default="wasm-bindgen")
args.add_argument("--toolchain", default="nightly-2026-11-26")
opts = args.parse_args()
out = opts.output.resolve()
out.mkdir(parents=True, exist_ok=True)
subprocess.run(
    ["cargo", "+" + opts.toolchain, "rustc", "--release", "--locked", "--target",
     "wasm32-unknown-unknown", "--target-dir", str(here / "target"), "--",
     "-C", "link-arg=-zstack-size=8388608"], cwd=here, check=True)
subprocess.run(
    [opts.wasm_bindgen, str(here / "target/wasm32-unknown-unknown/release/psms_web_inbox.wasm"),
     "--target", "web", "--out-dir", str(out / "wasm"), "--no-typescript"], check=True)
workflow = plistlib.loads((repo / "examples/Papamo SMS Encrypted.shortcut").read_bytes())
values = ["https://na.sms.papamo.dev/free/v1/messages",
          "PSMS_UPLOAD_".ljust(64, "_"), "PSMS_KEY_".ljust(32, "_")]
for action, value in zip(workflow["WFWorkflowActions"], values):
    action["WFWorkflowActionParameters"]["WFTextActionText"] = value
workflow["WFWorkflowImportQuestions"] = []
binary = plistlib.dumps(workflow, fmt=plistlib.FMT_BINARY, sort_keys=False)
for value in values[1:]:
    assert binary.count(value.encode("ascii")) == 1
(out / "workflow.plist").write_bytes(binary)
