# slim-images fixtures

`before.jsonl` → `after.jsonl` pair for the CC-parity golden test
(`cc_parity_strip_images_from_messages`).

## What it encodes

Three lines covering every place CC puts images or documents in its
session transcripts, per `compact.ts:145-199`
(`stripImagesFromMessages`):

1. **User message with a top-level image block**
   `message.content = [ {type:"image", source:{...base64...}} ]`
   → `[ {type:"text", text:"[image]"} ]`

2. **User message with a top-level document block**
   `message.content = [ {type:"document", source:{...base64...}} ]`
   → `[ {type:"text", text:"[document]"} ]`

3. **User message whose `tool_result` wraps an image and a document**
   `tool_result.content = [ {type:"text"...}, {type:"image"...}, {type:"document"...} ]`
   → inner image becomes `[image]` text stub; inner document becomes
   `[document]` text stub; the `tool_result` envelope
   (`tool_use_id`, `tool`, `is_error`) and any adjacent text blocks
   are preserved.

The session envelope fields (`uuid`, `parentUuid`, `sessionId`,
`timestamp`, `type`, `cwd`) are preserved byte-identical in intent —
though our transform goes through `serde_json::Value`, so key
ordering may drift (tolerated by CC's own loader).

## Generation procedure

The `after.jsonl` file was produced by applying the same
transformation CC applies in `stripImagesFromMessages`:

```ts
// paraphrase of src/services/compact/compact.ts:145 in claude_code_src
if (block.type === 'image')    return [{type:'text', text:'[image]'}]
if (block.type === 'document') return [{type:'text', text:'[document]'}]
// nested inside tool_result.content[*]:
if (item.type  === 'image')    return  {type:'text', text:'[image]'}
if (item.type  === 'document') return  {type:'text', text:'[document]'}
```

No other transformations. If CC's transform shape ever changes, this
fixture must be regenerated and the golden will fail loudly.

## Why these fixtures are normalized by serde_json

Our implementation parses each line with `serde_json::from_str` into a
`Value`, mutates, and serializes back. That means key ordering is
whatever serde_json's `serialize_map` emits (insertion-ordered
`Map<String,Value>`), not whatever the source file used.

For the parity assertion we compare parsed `Value`s, not raw bytes —
the `assert_eq!(got_lines, expected_lines)` in the test walks the
JSON tree. The fixtures can therefore be written in any
serde_json-roundtrip-safe form.
