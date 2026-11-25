"""Verify a populated SQLite cutover against the matrix job's disposable SQL server."""
import base64
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import sys
import time
import tempfile
import uuid
from urllib.request import Request, urlopen
from urllib.error import HTTPError

engine, connection_file = sys.argv[1:]
binary = str(Path("target/debug/psms").resolve())

with tempfile.TemporaryDirectory() as temporary:
    path = Path(temporary) / "archive.sqlite3"
    env = dict(os.environ, PSMS_DB_PATH=str(path), PSMS_BIND="127.0.0.1:0")

    def cli(*args, check=True):
        return subprocess.run([binary, *args], env=env, capture_output=True, text=True, check=check)

    def request(method, endpoint, token, payload=None):
        port = Path(str(path) + ".ready").read_text().strip()
        req = Request("http://" + port + endpoint, method=method,
                      headers={"Authorization":"Bearer " + token, "Content-Type":"application/json"},
                      data=None if payload is None else json.dumps(payload).encode())
        with urlopen(req, timeout=15) as response:
            return response.status, response.read()

    def message():
        return {"id":str(uuid.uuid4()), "sender":"papamo:encrypted:v1",
                "body":base64.b64encode(secrets.token_bytes(64)).decode(), "contact_id":"a"*64}

    replica_token=secrets.token_hex(32)
    env["PSMS_REPLICA_READ_TOKEN"]=replica_token
    pair=Path(temporary)/"pair.json"
    cli("sync","pair","--source","sa","--public-url","https://psms.example","--output",str(pair))
    peer=json.loads(pair.read_text())["token"]
    cli("init")
    try:
        upload = cli("token","upload").stdout.strip()
        owner = cli("token","owner").stdout.strip()
        first, second = message(), message()
        assert request("POST","/v1/messages",upload,first)[0] == 201
        url = "/v1/messages/" + first["id"]
        request("PATCH",url,owner,{"starred":True})
        assert cli("storage","switch",engine,"--connection-file",connection_file,check=False).returncode
        cli("stop")
        cli("storage","switch",engine,"--connection-file",connection_file)
        assert path.is_file()
        cli("init")
        cli("storage","status")
        assert cli("token","owner").stdout.strip()==owner
        body=json.loads(request("GET",url,owner)[1])
        assert body["body"]==first["body"] and body["starred"]
        assert request("POST","/v1/messages",upload,second)[0]==201
        assert request("POST","/v1/messages",upload,second)[0]==200
        contacts=json.loads(request("GET","/v1/contacts?include=messages",owner)[1])["contacts"]
        assert len(contacts)==1 and len(contacts[0]["messages"]["messages"])==2
        assert len(json.loads(cli("storage","desktop-page","0").stdout)["messages"])==2
        request("DELETE",url,owner)
        with sqlite3.connect(path) as old:
            assert old.execute("select count(*) from messages").fetchone()[0]==1
        cli("stop")
        path.rename(path.with_suffix(".backup"))
        cli("init")
        cli("storage","status")
        assert len(json.loads(request("GET","/v1/messages",owner)[1])["messages"])==1
        assert not path.exists()
        _, hook_data=request("POST","/v1/webhooks",owner,{"url":"https://127.0.0.1:1/event"})
        hook=json.loads(hook_data)["id"]
        another=message()
        request("POST","/v1/messages",upload,another)
        with sqlite3.connect(str(path)+".settings.sqlite3") as settings:
            for _ in range(20):
                queued=settings.execute("SELECT count(*) FROM webhook_deliveries WHERE endpoint=? AND message_id=? AND event='message.created'",(hook,another["id"])).fetchone()[0]
                if queued: break
                time.sleep(1)
            assert queued==1
        replica_path="/v1/replicas/sa/messages"
        change={"revision":1,"id":another["id"],"snapshot":dict(sender=another["sender"],body=another["body"],contact_id=another["contact_id"],starred=False,archived_at=100)}
        def deliver(change):
            for attempt in range(12):
                try:
                    return request("POST","/internal/sync/sa",peer,[change])
                except HTTPError as e:
                    if e.code!=503 or attempt==11: raise
                    time.sleep(2)
        assert json.loads(deliver(change)[1])==1
        assert json.loads(request("GET",replica_path,replica_token)[1])["messages"][0]["body"]==another["body"]
        deliver(dict(change,revision=2,snapshot=None))
        deliver(change)
        assert json.loads(request("GET",replica_path,replica_token)[1])["messages"]==[]
        container=subprocess.check_output(["docker","ps","-q","--filter","name=psms-integration-db"],text=True).strip()
        assert container
        subprocess.run(["docker","stop","-t","1",container],stdout=subprocess.DEVNULL,check=True)
        for method, endpoint, token, payload in [("GET","/readyz",owner,None),("POST","/v1/messages",upload,message())]:
            try:
                request(method,endpoint,token,payload)
            except HTTPError as error:
                assert error.code==503
            else:
                raise AssertionError("offline authoritative database accepted a request")
        assert not path.exists()
    except subprocess.CalledProcessError as error:
        print("CLI failed:", error.stderr, file=sys.stderr)
        raise
    finally:
        cli("stop",check=False)
