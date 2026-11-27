"""Create pSMS Check-in: a tiny Shortcut for a daily Time of Day automation. It reads the same
private phone setup as pSMS Capture and tells the inbox the phone is still running pSMS, so the
inbox can warn when iOS turns automations off."""
from pathlib import Path
import copy
import plistlib
import uuid

root = Path(__file__).resolve().parents[1]
capture = plistlib.loads((root / "examples/pSMS-Capture.shortcut").read_bytes())
actions = capture["WFWorkflowActions"]
# Setup text, its dictionary, and the endpoint and token values (the key isn't needed).
prefix = copy.deepcopy(actions[:6])
endpoint = prefix[3]["WFWorkflowActionParameters"]["UUID"]
post = copy.deepcopy(next(a for a in actions if a["WFWorkflowActionIdentifier"].endswith("downloadurl")))
replace = {
    "WFWorkflowActionIdentifier": "is.workflow.actions.text.replace",
    "WFWorkflowActionParameters": {
        "UUID": str(uuid.uuid4()).upper(),
        "WFInput": {"WFSerializationType": "WFTextTokenString", "Value": {"string": "￼",
            "attachmentsByRange": {"{0, 1}": {"Type": "ActionOutput", "OutputUUID": endpoint, "OutputName": "Text"}}}},
        "WFReplaceTextFind": "/messages",
        "WFReplaceTextReplace": "/checkin",
    },
}
params = post["WFWorkflowActionParameters"]
params["UUID"] = str(uuid.uuid4()).upper()
params["WFURL"] = {"WFSerializationType": "WFTextTokenString", "Value": {"string": "￼",
    "attachmentsByRange": {"{0, 1}": {"Type": "ActionOutput", "OutputUUID": replace["WFWorkflowActionParameters"]["UUID"], "OutputName": "Updated Text"}}}}
params["WFHTTPMethod"] = "POST"
params["WFHTTPBodyType"] = "JSON"
params["WFJSONValues"] = {"WFSerializationType": "WFDictionaryFieldValue", "Value": {"WFDictionaryFieldValueItems": []}}
workflow = {k: v for k, v in capture.items() if k not in ("WFWorkflowActions", "WFWorkflowInputContentItemClasses")}
workflow["WFWorkflowName"] = "pSMS Check-in"
workflow["WFWorkflowActions"] = prefix + [replace, post]
workflow["WFWorkflowInputContentItemClasses"] = []
(root / "examples/pSMS-Check-in.shortcut").write_bytes(plistlib.dumps(workflow, fmt=plistlib.FMT_BINARY, sort_keys=False))
print(root / "examples/pSMS-Check-in.shortcut")
