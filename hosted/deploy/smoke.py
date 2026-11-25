"""Run on the primary as operator, against its local API and real PostgreSQL."""
import os
import base64
import json
import secrets
from pathlib import Path
import subprocess
import uuid
import urllib.request
import urllib.error
from concurrent.futures import ThreadPoolExecutor

db_name=os.environ.get("PSMS_TEST_DATABASE","psms_accounts")
BASE="http://127.0.0.1:"+str(int(os.environ.get("PSMS_HOSTED_PORT",47833)))+"/free/v1/"
def call(path, data=None, token=None, method=None):
    headers={"Content-Type":"application/json"}
    if token:
        headers["Authorization"]="Bearer "+token
    req=urllib.request.Request(BASE+path,json.dumps(data).encode() if data is not None else None,headers,method=method)
    try:
        with urllib.request.urlopen(req,timeout=15) as res:
            raw=res.read()
            return res.status,json.loads(raw) if raw else None
    except urllib.error.HTTPError as exc:
        return exc.code,None

def sql(query):
    return subprocess.check_output(["runuser","-u","psms_hosted","--","psql","-XqAt","-v","ON_ERROR_STOP=1","-d",db_name],input=query,text=True).strip()

key=Path("/etc/psms/hosted-db/identity-key").read_text().strip()
users=[]
try:
    for _ in range(2):
        creds={"username":"same_test_display","subject":str(uuid.uuid4())}
        assert call("auth/exchange",creds)[0]==401
        status,account=call("auth/exchange",creds,key)
        assert status==200,status
        users.append((creds,account["token"]))
    (a,owner),(b,other)=users
    assert call("auth/login",{})[0]==404
    assert call("auth/register",{})[0]==404
    status,paired=call("pair",{},owner)
    assert status==200
    upload=paired["token"]
    msg={"id":str(uuid.uuid4()),"sender":"papamo:encrypted:v1","body":base64.b64encode(secrets.token_bytes(40)).decode(),"contact_id":"1"*64}
    assert call("messages",msg,upload)[0]==201
    assert call("messages",token=upload)[0]==401
    assert call("messages",token=other)[1]["messages"]==[]
    assert call("messages/"+msg["id"],token=other,method="DELETE")[0]==404
    assert call("messages",msg,other)[0]==201
    sql("INSERT INTO archived_messages(account_id,id,sender,body) SELECT a.id,gen_random_uuid()::text,'papamo:encrypted:v1','fixture' FROM accounts a CROSS JOIN generate_series(1,998) WHERE a.username='"+a["subject"]+"';")
    with ThreadPoolExecutor(2) as pool:
        results=list(pool.map(lambda _:call("messages",{**msg,"id":str(uuid.uuid4())},upload)[0],range(2)))
    assert sorted(results)==[201,507],results
    assert call("messages",msg,upload)[0]==200
    assert call("me",token=owner)[1]["used"]==1000
    assert call("messages/"+msg["id"],token=owner,method="DELETE")[0]==204
    assert call("messages",msg,upload)[0]==201
    if db_name.startswith("psms_qa_"):
        # Exercise role transitions in an isolated database with existing sessions.
        def grant(ttl):
            sql("INSERT INTO admin_role_snapshot VALUES(true,jsonb_build_array('"+a["subject"]+"'::text),now()+interval '"+ttl+"') ON CONFLICT(singleton) DO UPDATE SET subjects=excluded.subjects,expires_at=excluded.expires_at")
        grant("5 minutes")
        assert call("me",token=owner)[1]["limit"] is None
        assert call("me",token=other)[1]["limit"]==1000
        extra={**msg,"id":str(uuid.uuid4())}
        assert call("messages",extra,upload)[0]==201
        assert len(call("messages?offset=1000",token=owner)[1]["messages"])==1
        grant("-1 second")
        assert call("messages",{**msg,"id":str(uuid.uuid4())},upload)[0]==507
        grant("5 minutes")
        sql("UPDATE admin_role_snapshot SET subjects='[]'")
        assert call("me",token=owner)[1]=={"username":a["username"],"used":1001,"limit":1000}
        assert call("messages",extra,upload)[0]==200
        assert call("messages",{**msg,"id":str(uuid.uuid4())},upload)[0]==507
        print("PASS promotion, expiry, demotion, pagination and retry above the free limit")
    assert call("auth/session",token=owner,method="DELETE")[0]==204
    assert call("me",token=owner)[0]==401
    print("PASS subject exchange, logout, tenant isolation, upload scope, concurrent quota and idempotent retries")
finally:
    for creds,_ in users:
        sql("DELETE FROM accounts WHERE username='"+creds["subject"]+"';")
