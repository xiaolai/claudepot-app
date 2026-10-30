#!/bin/bash
# Rebuild the protocol-translation fixtures from a clai checkout
# (github.com/xiaolai/claude-driven-ai-providers) and compare them with the
# committed copies. See README.md in this directory for the file format.
#
#   capture.sh <clai-checkout>            rebuild into a temp dir, diff against
#                                         the committed files; exit 1 on any difference
#   capture.sh <clai-checkout> --update   rebuild and overwrite the committed files
#
# Exit status: 0 identical, 1 differences found, 2 capture error.
#
# Every input below is lifted from the clai test suites by line number, every
# filter is extracted from clai's own source by marker, and every expected
# output is what that filter actually printed. Each assertion is the suite's
# text verbatim: the script checks the text is present on the cited line, then
# runs it against the captured output. Requires bash, jq, awk, sed, diff, git.
set -euo pipefail
export LC_ALL=C

PINNED_COMMIT=8da773c2c74d491795245155ab14f1d791532612

die() { printf 'capture.sh: %s\n' "$*" >&2; exit 2; }
[ $# -ge 1 ] && [ $# -le 2 ] || die 'usage: capture.sh <clai-checkout> [--update]'
mode=check
if [ $# -eq 2 ]; then [ "$2" = --update ] || die "unknown option: $2"; mode=update; fi
for tool in jq awk sed diff git cmp; do command -v "$tool" >/dev/null || die "$tool is not on PATH"; done
clai=$(cd "$1" 2>/dev/null && pwd) || die "not a directory: $1"
[ -f "$clai/clcd" ] && [ -f "$clai/adapters/chat-request.jq" ] || die "not a clai checkout: $clai"
here=$(cd "$(dirname "$0")" && pwd)
case "$here" in */testdata/translate) ;; *) die "capture.sh must live in testdata/translate (found $here)";; esac

commit=$(git -C "$clai" rev-parse HEAD 2>/dev/null) || die "cannot read the clai commit"
if [ -n "$(git -C "$clai" status --porcelain 2>/dev/null)" ]; then commit="$commit-dirty"; fi
[ "$commit" = "$PINNED_COMMIT" ] || printf 'capture.sh: warning: clai is at %s, fixtures were pinned at %s\n' "$commit" "$PINNED_COMMIT" >&2
jq_version=$(jq --version)

work=$(mktemp -d "${TMPDIR:-/tmp}/translate-capture.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
out=$work/out
F=$out/filters
mkdir -p "$out/responses" "$out/chat" "$F" "$work/cases" "$work/tmp"
: >"$work/manifest.jsonl"

# ---------------------------------------------------------------- extraction
# quoted_after FILE PREFIX: the text after the unique PREFIX up to the next
# single quote — i.e. a single-quoted shell program, which cannot contain one.
quoted_after() {
  jq -Rrsj --arg p "$2" '
    if ([indices($p)[]]|length)!=1 then error("prefix is not unique in file: "+$p) else . end |
    .[(index($p)+($p|length)):] | if index("'"'"'")==null then error("unterminated quote after: "+$p) else .[:index("'"'"'")] end' "$1"
}
# quoted_on_line FILE LINE PREFIX: same, starting at the first PREFIX on line LINE
# (line numbers are those of the pinned commit; a moved line fails loudly).
quoted_on_line() {
  sed -n "${2}p" "$1" | jq -Rrsj --arg p "$3" --arg where "${1##*/}:$2" '
    if index($p)==null then error("prefix not on "+$where+": "+$p) else . end |
    .[(index($p)+($p|length)):] | if index("'"'"'")==null then error("unterminated quote on "+$where) else .[:index("'"'"'")] end'
}
# heredoc FILE OPENER: the body of the unique heredoc opened by OPENER.
heredoc() {
  jq -Rrsj --arg s "$2"$'\n' --arg e $'\nJQ\n' '
    if ([indices($s)[]]|length)!=1 then error("heredoc opener is not unique: "+$s) else . end |
    .[(index($s)+($s|length)):] | .[:index($e)] + "\n"' "$1"
}
lines() { sed -n "$2,$3p" "$clai/$1"; }   # lines FILE FIRST LAST (inclusive)

{ printf '# Extracted from clai clcd (request.jq heredoc). Do not edit; run capture.sh.\n'; heredoc "$clai/clcd" "cat >\"\$runtime/request.jq\" <<'JQ'"; } >"$F/clcd-request.jq"
{ printf '# Extracted from clai clcd (stream.jq heredoc). Do not edit; run capture.sh.\n'; heredoc "$clai/clcd" "cat >\"\$runtime/stream.jq\" <<'JQ'"; } >"$F/clcd-stream.jq"
{ printf '# Extracted from clai clcd handle(): non-streaming collection of a Responses stream.\n'; quoted_after "$clai/clcd" "--slurpfile names \"\$conn/reverse\" '"; printf '\n'; } >"$F/clcd-collect.jq"
{ printf '# Extracted from clai clcd handle(): wire tool names.\n'; quoted_on_line "$clai/clcd" 495 "jq '"; printf '\n'; } >"$F/clcd-names.jq"
{ printf '# Extracted from clai clcd handle(): reverse tool-name map.\n'; quoted_on_line "$clai/clcd" 496 "jq '"; printf '\n'; } >"$F/clcd-reverse.jq"
{ printf '# Extracted from clai clcd upstream_error(): upstream error body -> Anthropic error.\n'; quoted_after "$clai/clcd" "jq -Rsc --arg status \"\$upstream_status\" '"; printf '\n'; } >"$F/clcd-upstream-error.jq"
{ printf '# Extracted from clai clcd upstream_error(): Retry-After header.\n'; quoted_on_line "$clai/clcd" 445 "awk '"; printf '\n'; } >"$F/clcd-retry-after.awk"
{ printf '# Extracted from clai clcd http_error(): local error body.\n'; quoted_on_line "$clai/clcd" 433 "\"\$2\" '"; printf '\n'; } >"$F/clcd-http-error.jq"
{ printf '# Extracted from clai clcd --check: the probe request.\n'; quoted_on_line "$clai/clcd" 572 "--arg model \"\$model\" '"; printf '\n'; } >"$F/clcd-check-request.jq"
cp "$clai/adapters/chat-request.jq" "$F/chat-request.jq"
cp "$clai/adapters/chat-stream.jq" "$F/chat-stream.jq"
{ printf '# Extracted from clai tests/chat.sh: cache lines -> replay metadata.\n'; quoted_on_line "$clai/tests/chat.sh" 13 "jq -s '"; printf '\n'; } >"$F/chat-sh-metadata.jq"
{ printf '# Extracted from clai tests/bridge-api.sh mock claude: first Gemini request.\n'; quoted_on_line "$clai/tests/bridge-api.sh" 132 "\"\${CLAI_TEST_STREAM:-false}\" '"; printf '\n'; } >"$F/bridge-api-first-request.jq"
{ printf '# Extracted from clai tests/bridge-api.sh mock claude: tool_result replay request.\n'; quoted_after "$clai/tests/bridge-api.sh" "--slurpfile prior \"\$CLAI_TEST_STATE/first\" '"; printf '\n'; } >"$F/bridge-api-replay.jq"
{ printf '# Extracted from clai tests/bridge-api.sh collect_chat: client-side SSE reconstruction.\n'; quoted_after "$clai/tests/bridge-api.sh" "\"\$CLAI_TEST_STATE/chat-stream\"|jq -s '"; printf '\n'; } >"$F/bridge-api-client-collect.jq"
cat >"$F/emulated-metadata-lookup.jq" <<'JQ'
# NOT clai code. Emulates clcd prepare_chat() (clcd:327-343, bash): for each
# function_call in the normalized request, load the cached entry stored under
# (selected model, call id) — last write wins, as `mv -f` does — and require
# .id and .model to match. A missing entry is prepare_chat's `return 1`, which
# clcd turns into HTTP 400 (clcd:502-503).
# Input: the normalized Responses request. $cache: every cached entry, in write order.
.model as $model |
reduce (.input[]|select(.type=="function_call")|.call_id) as $id ({};
  ([$cache[]|select(.id==$id and .model==$model)]|last) as $entry |
  if $entry==null then error("no cached replay metadata for model "+$model+" and call "+$id)
  else .+{($id):$entry} end)
JQ
for f in "$F"/*; do [ -s "$f" ] || die "empty extraction: $f"; done

# ---------------------------------------------------------------- case DSL
FAMILY='' CASE='' CDIR=''
case_begin() { # FAMILY NAME ORIGIN COVERS
  FAMILY=$1 CASE=$2
  CDIR=$work/cases/$FAMILY/$CASE
  [ ! -e "$CDIR" ] || die "duplicate case $FAMILY/$CASE"
  mkdir -p "$CDIR"
  : >"$CDIR/steps.jsonl"; : >"$CDIR/asserts.jsonl"; : >"$CDIR/sources"; : >"$CDIR/notes"
  printf '%s' "$3" >"$CDIR/origin"; printf '%s' "$4" >"$CDIR/covers"
}
source_ref() { printf '%s\n' "$@" >>"$CDIR/sources"; }
note() { printf '%s\n' "$1" >>"$CDIR/notes"; }
fixture() { printf '%s/%s.%s' "$FAMILY" "$CASE" "$1"; }   # fixture SUFFIX -> relative path
# input_json SUFFIX: stdin -> pretty, key-sorted committed input; prints its relative path
input_json() { local rel; rel=$(fixture "$1"); jq -S . >"$out/$rel"; printf '%s' "$rel"; }
# input_raw SUFFIX: stdin -> byte-exact committed input; prints its relative path
input_raw() { local rel; rel=$(fixture "$1"); cat >"$out/$rel"; [ -s "$out/$rel" ] || die "empty input $rel"; printf '%s' "$rel"; }

# params_json JQ_ARGS...: record how a filter was invoked
params_json() {
  local p='{}'
  while [ $# -gt 0 ]; do
    case "$1" in
      --arg) p=$(jq -c --arg k "$2" --arg v "$3" '.arg[$k]=$v' <<<"$p"); shift 3;;
      --argjson) p=$(jq -c --arg k "$2" --argjson v "$3" '.argjson[$k]=$v' <<<"$p"); shift 3;;
      --slurpfile) p=$(jq -c --arg k "$2" --slurpfile v "$3" '.slurpfile[$k]=$v' <<<"$p"); shift 3;;
      -*) p=$(jq -c --arg f "$1" '.flags+=[$f]' <<<"$p"); shift;;
      *) die "unexpected jq argument: $1";;
    esac
  done
  printf '%s' "$p"
}
# resolve INPUT -> sets IN_REF (recorded) and IN_PATH (read); "@label" is a prior step, "-" is none
resolve() {
  case "$1" in
    -) IN_REF=null IN_PATH='';;
    @*) IN_PATH=$CDIR/${1#@}.out; [ -f "$IN_PATH" ] || die "$CASE: no step ${1#@}"; IN_REF=$(jq -cn --arg s "${1#@}" '{step:$s}');;
    *) IN_PATH=$out/$1; [ -f "$IN_PATH" ] || die "$CASE: no input $1"; IN_REF=$(jq -cn --arg f "${1##*/}" '{file:$f}');;
  esac
}
FRAMES='def frames: split("\n") | map(rtrimstr("\r")|select(.!="")) |
  map(if startswith("event: ") then {event:.[7:]}
      elif startswith("data: ") then (.[6:] as $d | {data:($d|try fromjson catch $d)})
      elif startswith("cache: ") then {cache:(.[7:]|fromjson)}
      else {line:.} end);'
# record_step LABEL KIND FILTER RC  (writes one steps.jsonl row from $CDIR/LABEL.{out,err})
record_step() {
  local label=$1 kind=$2 filter=$3 rc=$4 outf=$CDIR/$1.out errf=$CDIR/$1.err
  printf '%s' "$rc" >"$CDIR/$label.rc"
  case "$kind" in
    json)
      [ "$rc" -eq 0 ] || die "$FAMILY/$CASE/$label failed: $(cat "$errf")"
      jq -cn --arg l "$label" --arg f "$filter" --argjson i "$IN_REF" --argjson p "$PARAMS" --slurpfile o "$outf" \
        'if ($o|length)!=1 then error("expected one JSON value from "+$l) else . end | {label:$l,kind:"json",filter:$f,input:$i,params:$p,output:$o[0]}';;
    sse)
      [ "$rc" -eq 0 ] || die "$FAMILY/$CASE/$label failed: $(cat "$errf")"
      jq -cn --arg l "$label" --arg f "$filter" --argjson i "$IN_REF" --argjson p "$PARAMS" --rawfile o "$outf" \
        "$FRAMES"'{label:$l,kind:"sse",filter:$f,input:$i,params:$p,output_raw:$o,output_lines:($o|frames)}';;
    reject)
      [ "$rc" -ne 0 ] || die "$FAMILY/$CASE/$label was accepted but the case expects a rejection"
      # The message is cut exactly as clcd cuts jq errors (clcd:498).
      jq -cn --arg l "$label" --arg f "$filter" --argjson i "$IN_REF" --argjson p "$PARAMS" --rawfile o "$outf" \
        --arg e "$(sed 's/^jq: error.*): //' "$errf")" --argjson rc "$rc" \
        "$FRAMES"'{label:$l,kind:"reject",filter:$f,input:$i,params:$p,exit_code:$rc,error:$e,partial_output_raw:$o,partial_output_lines:($o|frames)}';;
    *) die "unknown step kind $kind";;
  esac >>"$CDIR/steps.jsonl"
}
# step LABEL KIND FILTER INPUT [JQ_ARGS...]  — run one extracted filter
step() {
  local label=$1 kind=$2 filter=$3 rc
  resolve "$4"; shift 4
  PARAMS=$(params_json "$@")
  set +e
  if [ -z "$IN_PATH" ]; then jq -n "$@" -f "$F/$filter" >"$CDIR/$label.out" 2>"$CDIR/$label.err"
  else jq "$@" -f "$F/$filter" "$IN_PATH" >"$CDIR/$label.out" 2>"$CDIR/$label.err"; fi
  rc=$?
  set -e
  record_step "$label" "$kind" "$filter" "$rc"
}
# glue LABEL INPUT DESCRIPTION CMD... — a shell transform clai performs between filters
glue() {
  local label=$1; resolve "$2"; local desc=$3; shift 3
  "$@" <"$IN_PATH" >"$CDIR/$label.out" || die "$CASE/$label glue failed"
  printf '0' >"$CDIR/$label.rc"
  jq -cn --arg l "$label" --arg d "$desc" --argjson i "$IN_REF" --rawfile o "$CDIR/$label.out" \
    '{label:$l,kind:"glue",glue:$d,input:$i,output_raw:$o}' >>"$CDIR/steps.jsonl"
}
# upstream_error LABEL STATUS BODY HEADERS API_KEY — clcd upstream_error() (clcd:435-447)
upstream_error() {
  local label=$1 status=$2 body=$3 headers=$4 api_key=$5 raw retry_after jbody
  raw=$(cat "$out/$body")
  [ -z "$api_key" ] || raw=${raw//"$api_key"/<redacted>}
  jbody=$(printf '%s' "$raw" | jq -Rsc --arg status "$status" -f "$F/clcd-upstream-error.jq") || die "$CASE: error filter failed"
  retry_after=$(awk -f "$F/clcd-retry-after.awk" "$out/$headers" 2>/dev/null || true)
  [[ "$retry_after" =~ ^[A-Za-z0-9,:\ -]*$ ]] || retry_after=''
  jq -cn --arg s "$status Codex" --arg r "$retry_after" --argjson b "$jbody" \
    '{status_line:$s,retry_after:(if $r=="" then null else $r end),body:$b}' >"$CDIR/$label.out"
  : >"$CDIR/$label.err"
  IN_REF=$(jq -cn --arg b "${body##*/}" --arg h "${headers##*/}" '{file:$b,headers_file:$h}')
  PARAMS=$(jq -cn --arg s "$status" --arg k "$api_key" '{arg:{status:$s},api_key_redacted_by_bash:$k}')
  record_step "$label" json "clcd-upstream-error.jq + clcd-retry-after.awk, bash glue as clcd:435-447" 0
}
# check STEP SUBJECT SRCFILE LINE TEXT [EXTRA...]
#   json      jq -e TEXT on the step output           EXTRA: jq args (--arg …)
#   json@PATH jq -e TEXT on (output | PATH)
#   data      the suite's `sed -n 's/^data: //p' | jq -se TEXT`
#   grep      grep -q -- TEXT on the raw output
#   rejected  TEXT is the suite's command that must fail; the step must be a reject
#   same      cmp with another step's output           EXTRA: OTHER_STEP
#   equiv     TEXT is not jq-runnable as written (a shell test, a digest
#             lookup); EXTRA: EQUIVALENT_JQ [--raw] [--step NAME STEP ...] is run
#             instead (--raw reads the step output as one string)
check() {
  local label=$1 subject=$2 file=$3 line=$4 text=$5 o rec equiv='' args='{}' other='' p
  shift 5
  o=$CDIR/$label.out
  [ -f "$o" ] || die "$CASE: check on unknown step $label"
  sed -n "${line}p" "$clai/$file" | grep -qF -- "$text" || die "$CASE: cited text is not on $file:$line: $text"
  case "$subject" in
    json) args=$(params_json "$@"); jq -e "$@" "$text" "$o" >/dev/null || die "$CASE: assertion failed ($file:$line): $text";;
    json@*) args=$(params_json "$@"); jq "${subject#json@}" "$o" | jq -e "$@" "$text" >/dev/null || die "$CASE: assertion failed ($file:$line): $text";;
    data) sed -n 's/^data: //p' "$o" | jq -se "$text" >/dev/null || die "$CASE: assertion failed ($file:$line): $text";;
    grep) grep -q -- "$text" "$o" || die "$CASE: assertion failed ($file:$line): $text";;
    rejected) [ "$(cat "$CDIR/$label.rc")" -ne 0 ] || die "$CASE: $label was not rejected";;
    same) other=$1; cmp -s "$o" "$CDIR/$other.out" || die "$CASE: $label differs from $other ($file:$line)";;
    equiv)
      equiv=$1; shift
      local jargs=() steps='{}'
      while [ $# -gt 0 ]; do
        case "$1" in
          --step) jargs+=(--slurpfile "$2" "$CDIR/$3.out"); steps=$(jq -c --arg k "$2" --arg v "$3" '.[$k]=$v' <<<"$steps"); shift 3;;
          --raw) jargs+=(-Rs); steps=$(jq -c '.["(raw output, jq -Rs)"]=true' <<<"$steps"); shift;;
          *) die "$CASE: unexpected equiv argument $1";;
        esac
      done
      jq -e ${jargs[@]+"${jargs[@]}"} "$equiv" "$o" >/dev/null || die "$CASE: equivalent assertion failed ($file:$line): $equiv"
      args=$(jq -cn --argjson s "$steps" 'if $s=={} then {} else {equivalent_inputs:$s} end');;
    *) die "unknown check subject $subject";;
  esac
  rec=$(jq -cn --arg st "$label" --arg src "$file:$line" --arg sub "$subject" --arg t "$text" --arg eq "$equiv" --arg ot "$other" --argjson a "$args" \
    '{step:$st,source:$src,subject:$sub,text:$t} + (if $eq!="" then {equivalent:$eq} else {} end) + (if $ot!="" then {other_step:$ot} else {} end) + (if $a!={} then {args:$a} else {} end)')
  printf '%s\n' "$rec" >>"$CDIR/asserts.jsonl"
}
case_end() { # KIND: expect | reject
  local kind=$1 dest
  dest=$out/$(fixture "$kind.json")
  [ -s "$CDIR/asserts.jsonl" ] || die "$CASE has no assertions"
  if [ "$kind" = reject ]; then jq -e -s 'any(.[];.kind=="reject")' "$CDIR/steps.jsonl" >/dev/null || die "$CASE: reject case without a reject step"; fi
  jq -S -n --arg fam "$FAMILY" --arg name "$CASE" --arg kind "$kind" --arg commit "$commit" \
    --rawfile origin "$CDIR/origin" --rawfile covers "$CDIR/covers" \
    --rawfile sources "$CDIR/sources" --rawfile notes "$CDIR/notes" \
    --slurpfile steps "$CDIR/steps.jsonl" --slurpfile asserts "$CDIR/asserts.jsonl" \
    '{case:$name,family:$fam,kind:$kind,origin:$origin,covers:$covers,
      source:{repo:"xiaolai/claude-driven-ai-providers",commit:$commit,lines:($sources|split("\n")|map(select(.!="")))},
      steps:$steps,assertions:$asserts}
     + (($notes|split("\n")|map(select(.!=""))) as $n | if ($n|length)>0 then {notes:$n} else {} end)' >"$dest"
  jq -cn --arg fam "$FAMILY" --arg name "$CASE" --arg kind "$kind" --rawfile origin "$CDIR/origin" --rawfile covers "$CDIR/covers" \
    '{family:$fam,case:$name,kind:$kind,origin:$origin,covers:$covers}' >>"$work/manifest.jsonl"
}

CP=tests/clcd-protocol.sh CR=tests/clcd-regression.sh BA=tests/bridge-api.sh CH=tests/chat.sh
tmp=$work/tmp
# Wire-name maps the clcd-protocol suite passes in (lines 11-12).
quoted_on_line "$clai/$CP" 11 "printf '" | sed 's/\\n$//' | jq -S . >"$tmp/cp-names.json"
quoted_on_line "$clai/$CP" 12 "printf '" | sed 's/\\n$//' | jq -S . >"$tmp/cp-reverse.json"
printf '{}\n' >"$tmp/empty.json"

# ================================================================ responses/
# --- clcd-protocol.sh: request translation
case_begin responses request-history-tools-image-thinking suite \
  'Anthropic request -> Responses: system to instructions, mid-turn system to developer, user image to input_image data URL, thinking block dropped, tool_use to function_call with wire name, error tool_result to "[Tool error]" output, tool_choice tool + disable_parallel_tool_use'
source_ref "$CP:11-19"
in=$(lines "$CP" 16 16 | input_json input.json)
step request json clcd-request.jq "$in" --arg model fallback --arg effort medium --slurpfile names "$tmp/cp-names.json"
check request json "$CP" 19 '.model=="gpt-6.1-sol" and .instructions=="test instructions" and .input[0].role=="developer" and .input[2].content[0].image_url=="data:image/png;base64,AA==" and .input[3].type=="function_call" and .input[3].name=="clcd_tool_0" and .input[4].output=="[Tool error]\nmissing" and .tools[0].name=="clcd_tool_0" and .tool_choice.name=="clcd_tool_0" and .parallel_tool_calls==false and .stream==true and .store==false'
note 'The assistant thinking block is absent from .input: clcd drops it (clcd:249). Reviewed exception, see README.'
case_end expect
base_input=responses/request-history-tools-image-thinking.input.json

case_begin responses reject-unsupported-content suite 'An unknown content block type is refused before dispatch'
source_ref "$CP:20-21"
in=$(quoted_on_line "$clai/$CP" 20 "printf '" | input_json input.json)
step request reject clcd-request.jq "$in" --arg model test --arg effort medium --slurpfile names "$tmp/cp-names.json"
check request rejected "$CP" 21 'if jq --arg model test --arg effort medium --slurpfile names filters/names.json -f filters/request.jq filters/bad.json'
case_end reject

for control in temperature top_p top_k output_format; do
  case_begin responses "reject-control-$control" suite "Request control $control is refused, not silently ignored"
  source_ref "$CP:42-45"
  in=$(jq --arg control "$control" '.+{($control):1}' "$out/$base_input" | input_json input.json)
  step request reject clcd-request.jq "$in" --arg model test --arg effort medium --slurpfile names "$tmp/cp-names.json"
  check request rejected "$CP" 44 'if jq --arg model test --arg effort medium --slurpfile names filters/names.json -f filters/request.jq filters/unsupported.json'
  note "Input is request-history-tools-image-thinking.input.json transformed by $CP:43: jq --arg control $control '.+{(\$control):1}'"
  case_end reject
done

case_begin responses reject-stop-sequences suite 'Non-empty stop_sequences are refused'
source_ref "$CP:46-47"
in=$(jq '.+{stop_sequences:["stop"]}' "$out/$base_input" | input_json input.json)
step request reject clcd-request.jq "$in" --arg model test --arg effort medium --slurpfile names "$tmp/cp-names.json"
check request rejected "$CP" 47 'if jq --arg model test --arg effort medium --slurpfile names filters/names.json -f filters/request.jq filters/unsupported.json'
note "Input is request-history-tools-image-thinking.input.json transformed by $CP:46: jq '.+{stop_sequences:[\"stop\"]}'"
case_end reject

case_begin responses request-adaptive-effort suite 'output_config.effort (adaptive thinking) becomes reasoning.effort'
source_ref "$CP:48"
in=$(jq '.+{output_config:{effort:"high"}}' "$out/$base_input" | input_json input.json)
step request json clcd-request.jq "$in" --arg model test --arg effort medium --slurpfile names "$tmp/cp-names.json"
check request json "$CP" 48 '.reasoning.effort=="high"'
case_end expect

case_begin responses request-effort-override suite 'An explicit bridge effort (CLCD_EFFORT) overrides the request effort'
source_ref "$CP:49"
step request json clcd-request.jq responses/request-adaptive-effort.input.json --arg model test --arg effort low --arg effort_override true --slurpfile names "$tmp/cp-names.json"
check request json "$CP" 49 '.reasoning.effort=="low"'
note 'Input is request-adaptive-effort.input.json.'
case_end expect

cp_roles='{"opus":"provider-pro","fable":"provider-pro","sonnet":"provider-standard","haiku":"provider-fast"}'
[ "$(quoted_on_line "$clai/$CP" 60 "--argjson mapping '")" = "$cp_roles" ] || die "$CP:60 mapping changed"
for requested in opus claude-opus-4-1 fable claude-fable-5-1 sonnet claude-3-5-sonnet haiku raw-provider-id; do
  # Expected values transcribed from the suite's case statement, clcd-protocol.sh:59.
  case "$requested" in opus|claude-opus*|fable|claude-fable*) expected=provider-pro;; sonnet|claude-3-5-sonnet) expected=provider-standard;; haiku) expected=provider-fast;; *) expected=$requested;; esac
  case_begin responses "request-model-role-$requested" suite "Requested model $requested resolves to $expected through the role mapping"
  source_ref "$CP:58-61"
  in=$(jq --arg requested "$requested" '.model=$requested' "$out/$base_input" | input_json input.json)
  step request json clcd-request.jq "$in" --arg model fallback --arg effort medium --argjson mapping "$cp_roles" --slurpfile names "$tmp/cp-names.json"
  check request json "$CP" 60 '.model==$expected' --arg expected "$expected"
  note "Input is request-history-tools-image-thinking.input.json with .model set to $requested ($CP:60)."
  case_end expect
done

# --- clcd-protocol.sh: stream translation
case_begin responses stream-text-and-tool suite 'Responses SSE -> Anthropic SSE: text part, function call with wire-name reversal, block indices, cached-token usage split'
source_ref "$CP:22-34"
ev=$(lines "$CP" 23 31 | input_raw events.sse)
step stream sse clcd-stream.jq "$ev" -Rnj --arg model test --slurpfile names "$tmp/cp-reverse.json"
check stream data "$CP" 34 '.[0].type=="message_start" and .[-1].type=="message_stop" and ([.[]|select(.type=="content_block_start")|.index]==[0,1]) and ([.[]|select(.type=="content_block_stop")|.index]==[0,1]) and any(.[]; .content_block.name=="mcp__long") and any(.[]; .delta.text=="Hello") and any(.[]; .delta.stop_reason=="tool_use" and .usage.input_tokens==70 and .usage.output_tokens==7)'
case_end expect

case_begin responses reject-truncated-stream suite 'A Responses stream that ends without a terminal event is an error, after message_start was already emitted'
source_ref "$CP:35"
ev=$(lines "$CP" 23 24 | input_raw events.sse)
step stream reject clcd-stream.jq "$ev" -Rnj --arg model test --slurpfile names "$tmp/cp-reverse.json"
check stream rejected "$CP" 35 'if head -n 2 filters/events | jq -Rnj --arg model test --slurpfile names filters/reverse.json -f filters/stream.jq'
note 'partial_output_raw is what a client has already received when the error fires (message_start).'
case_end reject

case_begin responses stream-refusal-crlf suite 'CRLF-framed stream with a refusal part becomes text, and [DONE] is tolerated'
source_ref "$CP:51-52"
fmt=$(quoted_on_line "$clai/$CP" 51 "printf '")
ev=$(printf "$fmt" | input_raw events.sse)   # the suite passes this string to printf as its format
step stream sse clcd-stream.jq "$ev" -Rnj --arg model test --slurpfile names "$tmp/cp-reverse.json"
check stream data "$CP" 52 'any(.[];.delta.text=="Cannot comply") and .[-1].type=="message_stop"'
case_end expect

fmt=$(quoted_on_line "$clai/$CP" 54 "printf '")
for terminal in response.incomplete response.failed error; do
  name=stream-terminal-${terminal#response.}
  case "$terminal" in
    response.incomplete) covers='response.incomplete maps to stop_reason max_tokens (every incomplete reason; reviewed exception)'; pattern='"stop_reason":"max_tokens"';;
    *) covers="$terminal becomes an Anthropic error event"; pattern='event: error';;
  esac
  case_begin responses "$name" suite "$covers"
  source_ref "$CP:53-56"
  ev=$(printf "$fmt" "$terminal" | input_raw events.sse)
  step stream sse clcd-stream.jq "$ev" -Rnj --arg model test --slurpfile names "$tmp/cp-reverse.json"
  check stream grep "$CP" 55 "$pattern"
  case_end expect
done

# --- bridge-api.sh: Responses routes through the real bridge glue
bridge_request() { # MODEL TRANSPORT MAPPING INPUT — names, reverse and request steps as clcd:495-497 runs them
  step names json clcd-names.jq "$4"
  step reverse json clcd-reverse.jq @names
  step request json clcd-request.jq "$4" --arg model "$1" --argjson mapping "$3" --arg effort medium --arg effort_override false --arg transport "$2" --slurpfile names "$CDIR/names.out"
}
api_ok_fmt=$(quoted_on_line "$clai/$BA" 43 "printf '")

case_begin responses bridge-openai-text suite 'OpenAI Responses route: request gets max_output_tokens and store:false; streamed text comes back'
source_ref "$BA:43" "$BA:129-130" "$BA:151"
in=$(quoted_on_line "$clai/$BA" 129 "--data '" | input_json input.json)
bridge_request fixture-openai openai '{}' "$in"
check request json "$BA" 41 '.store==false and .stream==true and .max_output_tokens>0'
ev=$(printf "$api_ok_fmt" | input_raw events.sse)
step stream sse clcd-stream.jq "$ev" -Rnj --arg model fixture-openai --slurpfile names "$CDIR/reverse.out"
check stream data "$BA" 130 'any(.[];.delta.text=="API_OK") and .[-1].type=="message_stop"'
note 'Bridge params (model fixture-openai, effort medium) confirmed against the upstream request clai actually sent in an instrumented bridge-api.sh run.'
case_end expect

case_begin responses bridge-grok-text suite 'Grok Responses route: same as OpenAI but the reasoning field is removed'
source_ref "$BA:42-43" "$BA:129-130" "$BA:152"
bridge_request grok-4.6 grok '{}' responses/bridge-openai-text.input.json
check request json "$BA" 41 '.store==false and .stream==true and .max_output_tokens>0'
check request json "$BA" 42 'has("reasoning")|not'
step stream sse clcd-stream.jq responses/bridge-openai-text.events.sse -Rnj --arg model grok-4.6 --slurpfile names "$CDIR/reverse.out"
check stream data "$BA" 130 'any(.[];.delta.text=="API_OK") and .[-1].type=="message_stop"'
note 'Input and events are bridge-openai-text.*. grok-4.6 is the providers.json default model the bridge selected.'
case_end expect

case_begin responses bridge-openai-mapping suite 'claude-opus-4-1 resolves through a profile role mapping; non-streaming collection of the reply'
source_ref "$BA:40-43" "$BA:127" "$BA:180-183"
in=$(quoted_on_line "$clai/$BA" 127 "--data '" | input_json input.json)
mapping=$(lines "$BA" 180 180 | sed "s/^printf '%s\\\\n' '//; s/' >.*//" | jq -c '.mapping.models')
bridge_request fixture-standard openai "$mapping" "$in"
check request json "$BA" 40 '.model=="fixture-pro"'
check request json "$BA" 41 '.store==false and .stream==true and .max_output_tokens>0'
step message json clcd-collect.jq responses/bridge-openai-text.events.sse -Rsc --arg model fixture-pro --slurpfile names "$CDIR/reverse.out"
check message json "$BA" 127 '.content[0].text=="API_OK"'
note 'The mapping is .mapping.models of the provider profile written at bridge-api.sh:180; events are bridge-openai-text.events.sse (the same mock reply, bridge-api.sh:43).'
case_end expect

case_begin responses bridge-openai-refusal suite 'A refusal in response.completed output is collected as text (non-streaming)'
source_ref "$BA:54" "$BA:116" "$BA:154"
in=$(quoted_on_line "$clai/$BA" 116 "--data '" | input_json input.json)
bridge_request gpt-6.1-sol openai '{}' "$in"
ev=$(printf "$(quoted_on_line "$clai/$BA" 54 "printf '")" | input_raw events.sse)
step message json clcd-collect.jq "$ev" -Rsc --arg model gpt-6.1-sol --slurpfile names "$CDIR/reverse.out"
check message json "$BA" 116 '.content[0].text=="Cannot comply"'
note 'No max_tokens in the request: clcd defaults max_output_tokens to 4096 (clcd:275).'
case_end expect

case_begin responses bridge-codex-check suite 'clcd --check probe on the Codex subscription route, collected non-streaming'
source_ref "$CR:45" "$CR:57" "$CR:86-91" "clcd:572-573"
in=$(jq -n --arg model legacy-codex -f "$F/clcd-check-request.jq" | input_json input.json)
bridge_request legacy-codex codex '{}' "$in"
ev=$( { printf "$(quoted_on_line "$clai/$CR" 45 "printf '")"
        printf "$(quoted_on_line "$clai/$CR" 57 "printf '")"; } | input_raw events.sse)
step message json clcd-collect.jq "$ev" -Rsc --arg model legacy-codex --slurpfile names "$CDIR/reverse.out"
check message json clcd 573 '[.content[] | select(.type=="text") | .text] | join("") | select(.=="CLCD_OK")'
note 'The assertion is clcd own --check acceptance test (clcd:573); clcd-regression.sh runs it in refresh mode (line 91).'
case_end expect

# --- upstream errors (clcd-regression.sh, bridge-api.sh)
case_begin responses upstream-error-quota suite 'HTTP 429 becomes rate_limit_error and Retry-After is forwarded'
source_ref "$CR:40" "$CR:70-71"
h=$(printf "$(quoted_on_line "$clai/$CR" 40 "quota) printf '")" | input_raw upstream-headers.txt)
b=$(printf "$(quoted_on_line "$clai/$CR" 40 '>"$headers"; printf '"'")" | input_raw upstream-body.json)
upstream_error mapped 429 "$b" "$h" ''
check mapped equiv "$CR" 70 '[ "$status" = 429 ]' '.status_line|startswith("429 ")'
check mapped equiv "$CR" 70 "grep -q '^Retry-After: 7\$'" '.retry_after=="7"'
check mapped json@.body "$CR" 71 '.error.type=="rate_limit_error"'
case_end expect

case_begin responses upstream-error-context suite 'A context-length 400 becomes invalid_request_error prefixed "prompt is too long:"'
source_ref "$CR:41" "$CR:73"
h=$(printf "$(quoted_on_line "$clai/$CR" 41 "context) printf '")" | input_raw upstream-headers.txt)
b=$(printf "$(quoted_on_line "$clai/$CR" 41 '>"$headers"; printf '"'")" | input_raw upstream-body.json)
upstream_error mapped 400 "$b" "$h" ''
check mapped equiv "$CR" 73 '[ "$status" = 400 ]' '.status_line|startswith("400 ")'
check mapped json@.body "$CR" 73 '.error.type=="invalid_request_error" and (.error.message|startswith("prompt is too long:"))'
case_end expect

case_begin responses upstream-error-auth-redaction suite 'HTTP 403 echoing the API key: status kept, key redacted, authentication_error'
source_ref "$BA:33-36" "$BA:118-121"
# bridge-api.sh uses a fresh `openssl rand -hex 16` key; this fixed all-zero placeholder stands in for it.
fake_key=00000000000000000000000000000000
h=$(printf "$(quoted_on_line "$clai/$BA" 34 "printf '")" | input_raw upstream-headers.txt)
b=$(printf '%s' "$fake_key" | jq -Rs "$(quoted_on_line "$clai/$BA" 35 "jq -Rs '")" | input_raw upstream-body.json)
upstream_error mapped 403 "$b" "$h" "$fake_key"
check mapped equiv "$BA" 119 '[ "$status" = 403 ]' '.status_line|startswith("403 ")'
check mapped equiv "$BA" 120 '! grep -q "$CLAI_TEST_KEY"' 'tojson|contains("00000000000000000000000000000000")|not'
check mapped json@.body "$BA" 121 '.error.message|contains("<redacted>")'
note 'The key is removed by bash substitution of the live key (clcd:438), not by the token regex in the jq filter: a translator must redact its own credential explicitly.'
case_end expect

# --- derived: reviewed exception, not a golden
case_begin responses exception-unfinished-tool-call derived \
  'REVIEWED EXCEPTION, NOT A GOLDEN: a function_call that never receives output_item.done is still emitted as a complete tool_use with input {} and stop_reason tool_use'
source_ref "$CP:23" "$CP:28" "$CP:31"
ev=$( { lines "$CP" 23 23; lines "$CP" 28 28; lines "$CP" 31 31; } | input_raw events.sse)
step stream sse clcd-stream.jq "$ev" -Rnj --arg model test --slurpfile names "$tmp/cp-reverse.json"
check stream data "$CP" 34 '.[0].type=="message_start" and .[-1].type=="message_stop"'
note 'Derived input: lines 23, 28 and 31 of clcd-protocol.sh, i.e. the suite stream with the argument delta and output_item.done removed. Not a suite case.'
note 'clai accepts this (clcd-stream.jq closes every open block on response.completed, clcd:313-316). Claudepot should refuse it; assert the opposite of this output.'
case_end expect

# ================================================================ chat/
# --- chat.sh: one chain, stream -> metadata -> replay request
case_begin chat roundtrip-chat-sh-signature suite \
  'Gemini chat stream with two parallel calls, a fragmented argument and a thought signature on one call -> normalized Responses events + cache entries -> replay request restoring extra_content and grouping both calls in one assistant message; CRLF framing gives identical output'
source_ref "$CH:6-19" "$CH:24-25"
ev=$(lines "$CH" 7 10 | input_raw events.sse)
step normalized sse chat-stream.jq "$ev" -Rnj --arg model fixture-model
check normalized data "$CH" 14 'any(.[];.type=="response.completed" and .response.usage.input_tokens==123 and (.response.output|length)==2) and ([.[]|select(.type=="response.output_item.done")]|length)==2'
glue cache @normalized "sed -n 's/^cache: //p' (chat.sh:13)" sed -n 's/^cache: //p'
step metadata json chat-sh-metadata.jq @cache -s
in=$(lines "$CH" 16 16 | input_json input.json)
step upstream_request json chat-request.jq "$in" --arg provider gemini --slurpfile metadata "$CDIR/metadata.out"
check upstream_request json "$CH" 19 '(.messages[2].tool_calls|length)==2 and .messages[2].tool_calls[0].extra_content.google.thought_signature=="fixture-signature-A" and .messages[2].tool_calls[1].extra_content=={} and .messages[3].tool_call_id=="call_one" and .reasoning_effort=="high" and .max_tokens==512'
crlf_awk=$(quoted_on_line "$clai/$CH" 24 "awk '")
ev2=$(awk "$crlf_awk" "$out/$ev" | input_raw crlf.events.sse)
step normalized_crlf sse chat-stream.jq "$ev2" -Rnj --arg model fixture-model
check normalized_crlf same "$CH" 25 'cmp "$tmp/normalized" "$tmp/crlf"' normalized
note 'input.json is the suite hand-written normalized (Responses-shaped) request, not an Anthropic request: chat-request.jq runs after clcd-request.jq in the bridge.'
case_end expect

case_begin chat reject-missing-metadata suite 'Gemini replay without the stored metadata is refused, never sent without the signature'
source_ref "$CH:20-21"
step upstream_request reject chat-request.jq chat/roundtrip-chat-sh-signature.input.json --arg provider gemini --slurpfile metadata "$tmp/empty.json"
check upstream_request rejected "$CH" 21 'if jq --arg provider gemini --slurpfile metadata "$tmp/missing" -f "$repo/adapters/chat-request.jq" "$tmp/request"'
note 'Input is roundtrip-chat-sh-signature.input.json; metadata is {} (chat.sh:20).'
case_end reject

case_begin chat reject-truncated-stream suite 'A chat stream that ends before a finish reason is an error'
source_ref "$CH:22"
ev=$(lines "$CH" 7 7 | input_raw events.sse)
step normalized reject chat-stream.jq "$ev" -Rnj --arg model fixture-model
check normalized rejected "$CH" 22 'if head -n 1 "$tmp/events"|jq -Rnj --arg model fixture-model -f "$repo/adapters/chat-stream.jq"'
case_end reject

# --- bridge-api.sh: the full Gemini tool round trip through clcd
case_begin chat roundtrip-gemini suite \
  'R2-7 chain on the Gemini route: Anthropic request -> Responses -> chat request; chat stream (parallel calls, fragmented argument, thought signature) -> cache + Anthropic message (collected and streamed); tool_result replay -> next chat request with call ids, arguments and extra_content restored; final text reply'
source_ref "$BA:57-64" "$BA:67-68" "$BA:73" "$BA:80-99" "$BA:131-145" "$BA:158-159" "$BA:173-174"
in=$(jq -n --argjson stream false -f "$F/bridge-api-first-request.jq" | input_json input.json)
# hop 1: client request -> upstream chat request (clcd handle + prepare_chat)
bridge_request fixture-gemini gemini '{}' "$in"
step metadata json emulated-metadata-lookup.jq @request --slurpfile cache "$tmp/empty.json"
step upstream_request json chat-request.jq @request --arg provider gemini --slurpfile metadata "$CDIR/metadata.out"
# hop 2: provider stream -> cache entries + Anthropic message
ev=$(lines "$BA" 57 64 | input_raw events.sse)
step normalized sse chat-stream.jq "$ev" -Rnj --arg model fixture-gemini
glue cache @normalized "normalize_chat: cache lines go to the private cache (clcd:350-355)" sed -n 's/^cache: //p'
glue responses_stream @normalized "normalize_chat: every other line is forwarded (clcd:356)" sed '/^cache: /d'
step message json clcd-collect.jq @responses_stream -Rsc --arg model fixture-gemini --slurpfile names "$CDIR/reverse.out"
check message json "$BA" 135 '.content[0].type=="tool_use" and .content[0].id=="gemini_call"'
step anthropic_stream sse clcd-stream.jq @responses_stream -Rnj --arg model fixture-gemini --slurpfile names "$CDIR/reverse.out"
check anthropic_stream data "$BA" 94 '.[-1].type=="message_stop" and (any(.[];.type=="error")|not)'
check anthropic_stream equiv "$BA" 88 'test -f "$XDG_CACHE_HOME/claude-driven-ai-providers/gemini/$digest.json"' \
  '[.|split("\n")[]|select(startswith("data: "))|.[6:]|fromjson|select(.type=="content_block_start" and .content_block.type=="tool_use")|.content_block.id] as $ids | ($ids|length)>0 and all($ids[]; . as $id | any($cache[]; .id==$id and .model=="fixture-gemini"))' \
  --raw --step cache cache
glue anthropic_payloads @anthropic_stream "sed -n 's/^data: //p' (bridge-api.sh:95)" sed -n 's/^data: //p'
step client_collected json bridge-api-client-collect.jq @anthropic_payloads -s
check client_collected json "$BA" 135 '.content[0].type=="tool_use" and .content[0].id=="gemini_call"'
check client_collected equiv "$BA" 133 'collect_chat >"$CLAI_TEST_STATE/first"' '.content==$m[0].content' --step m message
# hop 3: tool results -> next upstream chat request
step client_replay json bridge-api-replay.jq - --argjson stream false --slurpfile prior "$CDIR/message.out"
step replay_names json clcd-names.jq @client_replay
step replay_reverse json clcd-reverse.jq @replay_names
step replay_request json clcd-request.jq @client_replay --arg model fixture-gemini --argjson mapping '{}' --arg effort medium --arg effort_override false --arg transport gemini --slurpfile names "$CDIR/replay_names.out"
step replay_metadata json emulated-metadata-lookup.jq @replay_request --slurpfile cache "$CDIR/cache.out"
step replay_upstream_request json chat-request.jq @replay_request --arg provider gemini --slurpfile metadata "$CDIR/replay_metadata.out"
check replay_upstream_request json "$BA" 67 'any(.messages[];.tool_calls[0].extra_content.google.thought_signature=="fixture-private-signature" and (.tool_calls|length)==2 and .tool_calls[1].extra_content=={})'
# hop 4: the provider's final text reply
ev=$( { printf "$(quoted_on_line "$clai/$BA" 68 "printf '")"; printf "$(quoted_on_line "$clai/$BA" 73 "printf '")"; } | input_raw reply.events.sse)
step reply_normalized sse chat-stream.jq "$ev" -Rnj --arg model fixture-gemini
glue reply_stream @reply_normalized "normalize_chat (clcd:344-359)" sed '/^cache: /d'
step reply_message json clcd-collect.jq @reply_stream -Rsc --arg model fixture-gemini --slurpfile names "$CDIR/replay_reverse.out"
check reply_message json "$BA" 145 '.content[0].text=="CHAT_OK"'
note 'metadata and replay_metadata come from emulated-metadata-lookup.jq, a reproduction of the bash cache lookup in prepare_chat; everything else is clai jq.'
note 'Cross-checked: request, upstream_request and replay_upstream_request equal (jq -S) the bodies clai sent upstream in an instrumented bridge-api.sh run, and the cache entries equal its private cache files.'
case_end expect

case_begin chat reject-replay-missing-metadata suite 'The same replay under another model finds no metadata (scoped by model and call id) and is refused before any upstream call'
source_ref "$BA:141-143" "$BA:160-162" "clcd:331-339" "clcd:502-503"
in=$(jq -S . "$work/cases/chat/roundtrip-gemini/client_replay.out" | input_json input.json)
st=$(input_raw cache.jsonl <"$work/cases/chat/roundtrip-gemini/cache.out")
step request json clcd-request.jq "$in" --arg model different-gemini --argjson mapping '{}' --arg effort medium --arg effort_override false --arg transport gemini --slurpfile names "$tmp/empty.json"
step metadata reject emulated-metadata-lookup.jq @request --slurpfile cache "$out/$st"
step chat_adapter reject chat-request.jq @request --arg provider gemini --slurpfile metadata "$tmp/empty.json"
step http_error json clcd-http-error.jq - --arg message "$(quoted_on_line "$clai/clcd" 503 "http_error '400 Bad Request' '")"
check http_error json "$BA" 143 '.error.message|contains("metadata")'
check metadata rejected "$BA" 162 '[ "$(wc -l <"$CLAI_TEST_STATE/backend-calls")" = "$calls" ]'
note 'input.json is roundtrip-gemini client_replay; cache.jsonl is the private cache state roundtrip-gemini left behind (one entry per line, write order).'
note 'clcd returns HTTP 400 with http_error body when prepare_chat fails (clcd:502-503); no upstream request is made (bridge-api.sh:162).'
note 'names is {} here because the tool name read_file is already a valid wire name (clcd-names.jq would map it to itself).'
case_end reject

case_begin chat bridge-local-text suite 'Local chat route: no metadata lookup, no reasoning_effort, text reply collected'
source_ref "$BA:68" "$BA:73" "$BA:137-139" "$BA:163-166"
in=$(quoted_on_line "$clai/$BA" 137 "\"\${CLAI_TEST_STREAM:-false}\" '" | jq -n --argjson stream false -f /dev/stdin | input_json input.json)
bridge_request fixture-local local '{}' "$in"
step upstream_request json chat-request.jq @request --arg provider local --slurpfile metadata "$tmp/empty.json"
step reply_normalized sse chat-stream.jq chat/roundtrip-gemini.reply.events.sse -Rnj --arg model fixture-local
glue reply_stream @reply_normalized "normalize_chat (clcd:344-359)" sed '/^cache: /d'
step reply_message json clcd-collect.jq @reply_stream -Rsc --arg model fixture-local --slurpfile names "$CDIR/reverse.out"
check reply_message json "$BA" 139 '.content[0].text=="CHAT_OK"'
note 'Reply events are roundtrip-gemini.reply.events.sse (the same mock reply, bridge-api.sh:68,73).'
case_end expect

case_begin chat reject-malformed-stream suite 'Unparseable chat SSE data is an error, reported as a Chat stream problem'
source_ref "$BA:45" "$BA:107-114" "clcd:459-465"
ev=$(printf "$(quoted_on_line "$clai/$BA" 45 "printf '")" | input_raw events.sse)
step normalized reject chat-stream.jq "$ev" -Rnj --arg model gemini-3.8-flash
step http_error json clcd-http-error.jq - --arg message "$(quoted_on_line "$clai/clcd" 464 "else printf '")"
check normalized rejected "$BA" 113 '[ "$status" = 502 ]'
check http_error json "$BA" 113 '.error.message|contains("Chat")'
note 'stream_problem (clcd:459-465) picks the message by grepping the jq error; a parse error matches none of its three patterns, so it is the generic one. Streaming clients get it as an error event instead (bridge-api.sh:110).'
case_end reject

case_begin chat reject-incomplete-tool-call-arguments derived 'A tool call whose argument fragments never complete is refused at the finish reason'
source_ref "$CH:7" "$CH:9-10"
ev=$( { lines "$CH" 7 7; lines "$CH" 9 10; } | input_raw events.sse)
step normalized reject chat-stream.jq "$ev" -Rnj --arg model fixture-model
check normalized rejected "$CH" 22 'if head -n 1 "$tmp/events"|jq -Rnj --arg model fixture-model -f "$repo/adapters/chat-stream.jq"'
note 'Derived input: chat.sh lines 7, 9 and 10 (the second argument fragment removed). Not a suite case; the cited assertion is the nearest suite rejection.'
note 'The error is a jq parse error from fromjson (chat-stream.jq:40), not the "Incomplete chat tool call" message, so clcd reports the generic "malformed or interrupted" problem.'
case_end reject

# ---------------------------------------------------------------- manifest + compare
jq -S -s --arg commit "$commit" --arg jq "$jq_version" '
  {source:{repo:"xiaolai/claude-driven-ai-providers",commit:$commit},generated_with:$jq,
   counts:{total:length,
     by_family:(group_by(.family)|map({key:.[0].family,value:length})|from_entries),
     by_kind:(group_by(.kind)|map({key:.[0].kind,value:length})|from_entries),
     by_origin:(group_by(.origin)|map({key:.[0].origin,value:length})|from_entries)},
   cases:sort_by(.family,.case)}' "$work/manifest.jsonl" >"$out/manifest.json"

generated='responses chat filters manifest.json'
if [ "$mode" = update ]; then
  for item in $generated; do rm -rf -- "${here:?}/$item"; cp -R "$out/$item" "$here/$item"; done
  printf 'capture.sh: wrote %s cases to %s\n' "$(jq .counts.total "$out/manifest.json")" "$here"
  exit 0
fi
status=0
for item in $generated; do diff -r "$here/$item" "$out/$item" || status=1; done
if [ "$status" -eq 0 ]; then
  printf 'capture.sh: %s cases match the committed fixtures (clai %s, %s)\n' "$(jq .counts.total "$out/manifest.json")" "$commit" "$jq_version"
else
  printf 'capture.sh: fixtures differ from a fresh capture; rerun with --update after reviewing the diff\n' >&2
fi
exit "$status"
