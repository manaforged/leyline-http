from __future__ import annotations

from .toml_text import replace_list, replace_value

QPACK_SETTINGS = (1, 7)


def extension(capture: dict, name: str) -> dict:
    for ext in capture["tls"]["extensions"]:
        if ext.get("name") == name:
            return ext.get("data") or {}
    return {}


def algorithm_names(capture: dict, name: str) -> list[str]:
    return [alg["name"] for alg in extension(capture, name).get("supported_signature_algorithms", [])]


def transport_parameters(value: object) -> list[dict]:
    if isinstance(value, dict):
        if value.get("name") == "quic_transport_parameters":
            return (value.get("data") or {}).get("transport_parameters", [])
        value = list(value.values())
    if isinstance(value, list):
        return max((transport_parameters(item) for item in value), key=len, default=[])
    return []


def is_grease(version: int) -> bool:
    return version & 0x0F0F0F0F == 0x0A0A0A0A


def fingerprint(capture: dict) -> dict:
    params = [
        (param.get("id"), [v["id"] for v in param.get("available_versions", []) if not is_grease(v["id"])])
        for param in transport_parameters(capture["tls"])
    ]
    return {"ja4_r": capture["ja4_r"].split("_"), "h3_text": capture["h3_text"], "params": params}


def qpack(capture: dict) -> tuple[int, int]:
    settings = dict(item.split(":", 1) for item in capture["h3_text"].split("|", 1)[0].split(";"))
    first, second = (int(settings.get(str(key), 0)) for key in QPACK_SETTINGS)
    return first, second


def apply_h3(text: str, runs: list[dict], previous: list[dict]) -> tuple[str, list[str], list[str]]:
    prints = [fingerprint(run) for run in runs]
    if any(fp != prints[0] for fp in prints[1:]):
        return text, [], ["the HTTP/3 runs disagree with each other"]
    if not previous:
        return text, [], ["no HTTP/3 capture of the previous version to compare against"]
    new, old = prints[0], fingerprint(previous[0])
    if new == old:
        return text, ["HTTP/3 matches the previous version"], []
    sigalgs_only = (
        new["ja4_r"][:3] == old["ja4_r"][:3]
        and new["h3_text"] == old["h3_text"]
        and new["params"] == old["params"]
    )
    if not sigalgs_only:
        changed = [key for key in new if new[key] != old[key]]
        return text, [], [f"HTTP/3 changed beyond signature algorithms: {', '.join(changed)}"]
    text = replace_list(text, "h3.tls", "sigalgs", algorithm_names(runs[0], "signature_algorithms"))
    delegated = ":".join(algorithm_names(runs[0], "delegated_credential"))
    text = replace_value(text, "h3.tls", "delegated_credentials", f'"{delegated}"')
    return text, ["HTTP/3 signature algorithms and delegated credentials follow the capture"], []
