"""Create a shareable Shortcut with one import question for private phone setup."""
from pathlib import Path
import plistlib
import uuid

root = Path(__file__).resolve().parents[1]
workflow = plistlib.loads((root / "examples/Papamo SMS Encrypted.shortcut").read_bytes())
actions = workflow["WFWorkflowActions"]
def action(name, **fields):
    return {"WFWorkflowActionIdentifier": "is.workflow.actions." + name,
            "WFWorkflowActionParameters": dict(UUID=str(uuid.uuid4()).upper(), **fields)}
def ref(step, name):
    return {"WFSerializationType":"WFTextTokenAttachment","Value":{
        "Type":"ActionOutput","OutputName":name,"OutputUUID":step["WFWorkflowActionParameters"]["UUID"]}}
setup = action("gettext", WFTextActionText="{}")
dictionary = action("detect.dictionary", WFInput=ref(setup, "Text"))
prefix = [setup, dictionary]
for original, field in zip(actions[:3], ("endpoint", "token", "key")):
    value = action("getvalueforkey", WFInput=ref(dictionary,"Dictionary"),
                   WFGetDictionaryValueType="Value", WFDictionaryKey=field)
    original["WFWorkflowActionParameters"]["WFTextActionText"] = {
        "WFSerializationType":"WFTextTokenString", "Value":{"string":"\ufffc",
        "attachmentsByRange":{"{0, 1}":ref(value,"Dictionary Value")["Value"]}}}
    prefix.extend((value,original))
workflow["WFWorkflowName"] = "pSMS Capture"
workflow["WFWorkflowActions"] = prefix + actions[3:]
workflow["WFWorkflowImportQuestions"] = [{"ActionIndex":0,"Category":"Parameter",
    "ParameterKey":"WFTextActionText","DefaultValue":"",
    "Text":"Paste the private phone setup copied from pSMS."}]
(root/"examples/pSMS-Capture.shortcut").write_bytes(plistlib.dumps(workflow,fmt=plistlib.FMT_BINARY,sort_keys=False))
