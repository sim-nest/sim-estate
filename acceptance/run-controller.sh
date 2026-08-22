#!/bin/sh
set -eu

schema=sim.estate-acceptance/v1
manifest=acceptance/controller-v1.sx
harness=acceptance/run-controller.sh

usage() {
  echo "usage: $0 capture --source COMMIT --target control:local --output FILE | verify --source COMMIT FILE" >&2
  exit 2
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

sha256_text() {
  printf '%s' "$1" | sha256sum | awk '{print $1}'
}

field() {
  name=$1
  value=$2
  case "$value" in
    *\"* | *\\*) echo "field contains an unsafe character" >&2; exit 1 ;;
  esac
  printf '  (%s "%s")\n' "$name" "$value"
}

case_line() {
  id=$1
  category=$2
  printf '    (case (id "%s") (category "%s") (passed true))\n' "$id" "$category"
}

require_path() {
  value=$1
  label=$2
  test -e "$value" || {
    echo "$label is unavailable" >&2
    exit 1
  }
}

git_state_digest() {
  root=$1
  head=$(git -C "$root" rev-parse HEAD)
  index=$(git -C "$root" status --porcelain=v1 -z --untracked-files=all | sha256sum | awk '{print $1}')
  printf '%s:%s' "$head" "$index" | sha256sum | awk '{print $1}'
}

binding_summary() {
  python3 - "$1" <<'PY'
import hashlib
import json
import sys
import tomllib
from pathlib import Path

path = Path(sys.argv[1])
data = tomllib.loads(path.read_text(encoding="utf-8"))
project = data["project"][0]
operations = {item["literal"]: item for item in project["operation"]}
ping = operations["ping"]
assert data["schema"] == "sim.estate-private-bindings/v1"
assert project["provider_id"] == "estate/provider/ansible"
assert project["inventory_program_ref"] == "boot/tool/ansible-inventory"
assert project["inventory_arguments"] == ["--list"]
assert project["make_program_ref"] == "boot/tool/make"
assert project["target_encoder"] == "estate/encoder/exact-inventory-member"
assert ping["mode"] == "Inspect"
assert ping["capability"] == "estate.observe"
assert ping["risk_floor"] == "risk/read"
assert ping["literal"] == "ping"
assert ping["callback"] == "completion-only"
payload = {
    "schema": data["schema"],
    "project": project["id"],
    "provider": project["provider_id"],
    "operation": ping["id"],
    "literal": ping["literal"],
    "mode": ping["mode"],
    "capability": ping["capability"],
    "callback": ping["callback"],
    "max_targets": ping["max_targets"],
    "max_events": ping["max_events"],
    "max_duration_ticks": ping["max_duration_ticks"],
    "max_value_bytes": ping["max_value_bytes"],
}
print(hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
PY
}

write_event_book() {
  path=$1
  source=$2
  operation=$3
  result=$4
  {
    echo "(sim.estate-controller-events/v1"
    field source "$source"
    field operation "$operation"
    field accepted "true"
    field final "$result"
    field raw_output_retained "false"
    echo ")"
  } >"$path"
}

assert_source() {
  case "$1" in
    *[!0-9a-f]* | "" ) echo "source must be a lowercase hex commit" >&2; exit 1 ;;
  esac
  test "${#1}" -eq 40 || {
    echo "source must be a full commit" >&2
    exit 1
  }
}

verify() {
  source=$1
  artifact=$2
  assert_source "$source"
  test "$(sed -n '1p' "$artifact")" = "($schema"
  test "$(sed -n '$p' "$artifact")" = ")"
  grep -Fq "  (source \"$source\")" "$artifact"
  grep -Fq '  (target "control-local")' "$artifact"
  grep -Fq '  (controller_capsule "physical-readonly")' "$artifact"
  grep -Fq '  (operation "estate/ping")' "$artifact"
  grep -Fq '  (private_boundary "host-blind")' "$artifact"
  grep -Fq '  (artifact_retention "sanitized-only")' "$artifact"
  grep -Fq '  (network "lan-bounded")' "$artifact"
  grep -Fq '  (project_unchanged "true")' "$artifact"
  grep -Fq '  (callback_final "succeeded")' "$artifact"
  grep -Fq '  (result "physical-controller-readonly-pass")' "$artifact"
  for required in controller-capability staged-source binding-registry inventory-discovery readonly-operation callback-reconcile project-equivalence private-boundary offline-verifier; do
    grep -Fq "(case (id \"$required\")" "$artifact"
  done
  for digest_field in manifest_sha256 harness_sha256 binding_summary_sha256 inventory_sha256 operation_output_sha256 event_book_sha256 project_before_sha256 project_after_sha256 target_set_sha256; do
    line=$(sed -n "s/^  ($digest_field \"\\([0-9a-f][0-9a-f]*\\)\")$/\\1/p" "$artifact")
    test "${#line}" -eq 64 || {
      echo "artifact has invalid $digest_field" >&2
      exit 1
    }
  done
  if grep -Eiq 'hostname|username|user=|serial|uuid|ssh|known_hosts|inventory_(path|body|value)|hostvars|vault|/home/|\\\\' "$artifact"; then
    echo "acceptance artifact contains private identity data" >&2
    exit 1
  fi
}

capture() {
  source=$1
  target=$2
  output=$3
  private_root=${SIM_ESTATE_PRIVATE_ROOT:-}
  binding_registry=${SIM_ESTATE_BINDING_REGISTRY:-}
  observe_limit=${SIM_ESTATE_OBSERVE_LIMIT:-}
  assert_source "$source"
  test "$target" = "control:local" || {
    echo "unsupported controller target" >&2
    exit 1
  }
  test -n "$private_root" || {
    echo "SIM_ESTATE_PRIVATE_ROOT is required" >&2
    exit 1
  }
  test -n "$binding_registry" || {
    echo "SIM_ESTATE_BINDING_REGISTRY is required" >&2
    exit 1
  }
  test -n "$observe_limit" || {
    echo "SIM_ESTATE_OBSERVE_LIMIT is required" >&2
    exit 1
  }
  require_path "$private_root/Makefile" "private project"
  require_path "$binding_registry" "binding registry"
  test -f "$manifest"
  test -f "$harness"
  mkdir -p "$(dirname "$output")"
  work=$(mktemp -d "${TMPDIR:-/tmp}/sim-estate-controller.XXXXXX")
  trap 'test -z "${work:-}" || rm -rf "$work"' EXIT
  tmp="${output}.tmp.$$"
  before=$(git_state_digest "$private_root")
  binding_digest=$(binding_summary "$binding_registry")
  inventory_raw="$work/inventory.json"
  operation_stdout="$work/operation.stdout"
  operation_stderr="$work/operation.stderr"
  event_book="$work/events.sx"
  (
    cd "$private_root"
    ANSIBLE_RETRY_FILES_ENABLED=False \
      ANSIBLE_LOCAL_TEMP="$work/local-tmp" \
      ansible-inventory --list >"$inventory_raw"
  )
  ANSIBLE_RETRY_FILES_ENABLED=False \
    ANSIBLE_LOCAL_TEMP="$work/local-tmp" \
    make -C "$private_root" ping LIMIT="$observe_limit" >"$operation_stdout" 2>"$operation_stderr"
  after=$(git_state_digest "$private_root")
  test "$before" = "$after" || {
    echo "private project changed during read-only observation" >&2
    exit 1
  }
  write_event_book "$event_book" "$source" "estate/ping" "succeeded"
  {
    echo "($schema"
    field source "$source"
    field target "control-local"
    field controller_capsule "physical-readonly"
    field operation "estate/ping"
    field private_boundary "host-blind"
    field artifact_retention "sanitized-only"
    field network "lan-bounded"
    field manifest_sha256 "$(sha256_file "$manifest")"
    field harness_sha256 "$(sha256_file "$harness")"
    field binding_summary_sha256 "$binding_digest"
    field inventory_sha256 "$(sha256_file "$inventory_raw")"
    field operation_output_sha256 "$(cat "$operation_stdout" "$operation_stderr" | sha256sum | awk '{print $1}')"
    field event_book_sha256 "$(sha256_file "$event_book")"
    field project_before_sha256 "$before"
    field project_after_sha256 "$after"
    field target_set_sha256 "$(sha256_text "$observe_limit")"
    field project_unchanged "true"
    field callback_final "succeeded"
    echo "  (cases"
    case_line controller-capability resource/controller
    case_line staged-source source/exact-clean
    case_line binding-registry binding/sealed
    case_line inventory-discovery inventory/sanitized-digest
    case_line readonly-operation operation/read-only
    case_line callback-reconcile callback/completion-only
    case_line project-equivalence project/before-after
    case_line private-boundary evidence/sanitized
    case_line offline-verifier artifact/verify
    echo "  )"
    field result physical-controller-readonly-pass
    echo ")"
  } >"$tmp"
  verify "$source" "$tmp"
  mv "$tmp" "$output"
}

case "${1:-}" in
  capture)
    shift
    source=
    target=
    output=
    while test "$#" -gt 0; do
      case "$1" in
        --source) source=${2:-}; shift 2 ;;
        --target) target=${2:-}; shift 2 ;;
        --output) output=${2:-}; shift 2 ;;
        *) usage ;;
      esac
    done
    test -n "$source" && test -n "$target" && test -n "$output" || usage
    capture "$source" "$target" "$output"
    ;;
  verify)
    shift
    test "${1:-}" = "--source" || usage
    source=${2:-}
    artifact=${3:-}
    test -n "$source" && test -n "$artifact" || usage
    verify "$source" "$artifact"
    ;;
  *)
    usage
    ;;
esac
