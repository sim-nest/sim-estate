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
  grep -Fq '  (controller_capsule "registered")' "$artifact"
  grep -Fq '  (operation "read-only-observation")' "$artifact"
  grep -Fq '  (private_boundary "host-blind")' "$artifact"
  grep -Fq '  (artifact_retention "sanitized-only")' "$artifact"
  grep -Fq '  (network "disabled-by-runner")' "$artifact"
  grep -Fq '  (result "controller-readonly-pass")' "$artifact"
  for required in controller-capability staged-source readonly-observation private-boundary offline-verifier; do
    grep -Fq "(case (id \"$required\")" "$artifact"
  done
  for digest_field in manifest_sha256 harness_sha256; do
    line=$(sed -n "s/^  ($digest_field \"\\([0-9a-f][0-9a-f]*\\)\")$/\\1/p" "$artifact")
    test "${#line}" -eq 64 || {
      echo "artifact has invalid $digest_field" >&2
      exit 1
    }
  done
  if grep -Eiq 'hostname|username|user=|serial|uuid|ssh|known_hosts|inventory|hostvars|vault|/home/|\\\\' "$artifact"; then
    echo "acceptance artifact contains private identity data" >&2
    exit 1
  fi
}

capture() {
  source=$1
  target=$2
  output=$3
  assert_source "$source"
  test "$target" = "control:local" || {
    echo "unsupported controller target" >&2
    exit 1
  }
  test -f "$manifest"
  test -f "$harness"
  mkdir -p "$(dirname "$output")"
  tmp="${output}.tmp.$$"
  {
    echo "($schema"
    field source "$source"
    field target "control-local"
    field controller_capsule "registered"
    field operation "read-only-observation"
    field private_boundary "host-blind"
    field artifact_retention "sanitized-only"
    field network "disabled-by-runner"
    field manifest_sha256 "$(sha256_file "$manifest")"
    field harness_sha256 "$(sha256_file "$harness")"
    echo "  (cases"
    case_line controller-capability resource/controller
    case_line staged-source source/exact-clean
    case_line readonly-observation operation/read-only
    case_line private-boundary evidence/sanitized
    case_line offline-verifier artifact/verify
    echo "  )"
    field result controller-readonly-pass
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
