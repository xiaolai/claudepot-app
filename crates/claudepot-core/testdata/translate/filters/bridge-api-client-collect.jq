# Extracted from clai tests/bridge-api.sh collect_chat: client-side SSE reconstruction.
. as $events |
    {content:([$events[]|select(.type=="content_block_start" and .content_block.type=="tool_use")|. as $start|
      .content_block+{input:([$events[]|select(.type=="content_block_delta" and .index==$start.index)|.delta.partial_json//empty]|join("")|fromjson)}] +
      ([$events[]|select(.type=="content_block_delta")|.delta.text//empty]|join("")|if .=="" then [] else [{type:"text",text:.}] end))}
