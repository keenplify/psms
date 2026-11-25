"""Verify a populated SQLite cutover against the matrix job's disposable SQL server."""
import base64
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import sys
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
    except subprocess.CalledProcessError as error:
        print("CLI failed:", error.stderr, file=sys.stderr)
        raise
    finally:
        cli("stop",check=False)
