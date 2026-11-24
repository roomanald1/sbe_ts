import json
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent

def catalogue():
    return json.loads(
        (HERE / "catalogue.json").read_text()
    )

def query(service, endpoint, max_bytes=4000000, since=None, params=None):
    routes = catalogue()

    if service not in routes:
        raise RuntimeError("Unknown service: " + service)

    if endpoint not in routes[service]:
        raise RuntimeError("Unknown endpoint: " + endpoint)

    if not 1 <= max_bytes <= 64000000:
        raise RuntimeError("Invalid response size limit")

    request = {
        "service": service,
        "route": routes[service][endpoint],
        "max_bytes": max_bytes,
        **({"since": since} if since is not None else {})
    }
    if params:
        if (service, endpoint) != ("positions", "trades"):
            raise RuntimeError("Query parameters are only supported for positions.trades")
        request["params"] = params

    result = subprocess.run(
        ["node", str(HERE / "rsocket_client.cjs")],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        timeout=15
    )

    if result.returncode:
        raise RuntimeError(result.stderr.strip()[:1000])

    data = json.loads(result.stdout)
    response = data["response"]

    if not isinstance(response, dict):
        raise RuntimeError("Unexpected response envelope")

    envelope = (
        response.get("response")
        if service == "learning"
        else response
    )

    if not isinstance(envelope, dict):
        raise RuntimeError("Missing application envelope")

    status = envelope.get("status")

    if type(status) is not int or not 200 <= status < 300:
        raise RuntimeError(
            f"Application status {status}: "
            + str(envelope.get("body", ""))[:300]
        )

    if "body" not in envelope:
        raise RuntimeError("Missing response body")

    data["application_status"] = status
    data["body"] = envelope["body"]

    return data

def shape(value, depth=0):
    if depth >= 5:
        return type(value).__name__

    if isinstance(value, dict):
        return {
            k: shape(v, depth+1)
            for k,v in list(value.items())[:40]
        }

    if isinstance(value, list):
        return {
            "type": "array",
            "length": len(value),
            "item": shape(value[0], depth+1)
                    if value else None
        }

    return type(value).__name__

def emit(data, output=None):
    raw = json.dumps(data, indent=2)

    if output:
        with Path(output).open("x") as f:
            f.write(raw + "\n")
        print("Evidence:", output)
        return

    if len(raw.encode()) <= 8000:
        print(raw)
        return

    with tempfile.NamedTemporaryFile(
        mode="w",
        prefix="sbe-api-",
        suffix=".json",
        delete=False
    ) as f:
        f.write(raw)
        filename = f.name

    print(json.dumps({
        "evidence": filename,
        "bytes": len(raw.encode()),
        "service": data.get("service"),
        "route": data.get("route"),
        "application_status": data.get("application_status"),
        "response_shape": shape(data.get("body"))
    }, indent=2)[:8000])

def field(value, key):
    for part in key.split("."):
        if not isinstance(value, dict) or part not in value:
            raise KeyError("Missing field: " + key)
        value = value[part]
    return value
