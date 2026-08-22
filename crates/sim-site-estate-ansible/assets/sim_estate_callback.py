"""SIM Estate aggregate callback: bounded metadata only, no result payloads."""
import hashlib
import json
import os

SCHEMA = 1
_sequence = 0
_prior = "0" * 64

def _opaque(kind, value):
    return kind + "/" + hashlib.sha256(str(value).encode("utf-8")).hexdigest()[:32]

def _emit(status, host=None, task=None, changed=False, terminal=False):
    global _sequence, _prior
    record = {"schema": SCHEMA, "run": os.environ["SIM_ESTATE_RUN"],
              "plan": os.environ["SIM_ESTATE_PLAN"], "sequence": _sequence,
              "host": _opaque("host", host) if host is not None else None,
              "task": _opaque("task", task) if task is not None else None,
              "status": status, "changed": bool(changed), "prior_hash": _prior,
              "current_hash": "", "terminal": bool(terminal)}
    canonical = json.dumps(record, sort_keys=True, separators=(",", ":")).encode("utf-8")
    record["current_hash"] = hashlib.sha256(canonical).hexdigest()
    line = json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n"
    if len(line.encode("utf-8")) > 8192:
        raise RuntimeError("SIM Estate callback line exceeds bound")
    with open(os.environ["SIM_ESTATE_EVENTS"], "a", encoding="utf-8") as output:
        output.write(line)
        output.flush()
    _prior = record["current_hash"]
    _sequence += 1

class CallbackModule:
    CALLBACK_VERSION = 2.0
    CALLBACK_TYPE = "aggregate"
    CALLBACK_NAME = "sim_estate_aggregate"
    CALLBACK_NEEDS_WHITELIST = True
    def v2_playbook_on_start(self, playbook): _emit("accepted")
    def v2_runner_on_ok(self, result): _emit("ok", result._host.get_name(), result._task.get_name(), bool(result._result.get("changed", False)))
    def v2_runner_on_failed(self, result, ignore_errors=False): _emit("failed", result._host.get_name(), result._task.get_name())
    def v2_playbook_on_stats(self, stats): _emit("final", changed=any(stats.changed.values()), terminal=True)
