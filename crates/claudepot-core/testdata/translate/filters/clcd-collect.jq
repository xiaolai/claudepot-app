# Extracted from clai clcd handle(): non-streaming collection of a Responses stream.

      [split("\n")[] | select(startswith("data: ")) | .[6:] | select(.!="[DONE]") | fromjson | select(.type=="response.completed" or .type=="response.incomplete")][-1].response |
      if .==null then error("Incomplete Codex stream") else . end |
      {id:.id,type:"message",role:"assistant",model:$model,stop_sequence:null,
       stop_reason:(if .status=="incomplete" then "max_tokens" elif any(.output[]; .type=="function_call") then "tool_use" else "end_turn" end),
       content:[.output[] | if .type=="message" then .content[] | select(.type|IN("output_text","refusal")) | {type:"text",text:(.text//.refusal)}
         elif .type=="function_call" then {type:"tool_use",id:.call_id,name:($names[0][.name] // .name),input:(.arguments|fromjson)} else empty end],
       usage:{input_tokens:((.usage.input_tokens // 0)-(.usage.input_tokens_details.cached_tokens // 0)),cache_read_input_tokens:(.usage.input_tokens_details.cached_tokens // 0),cache_creation_input_tokens:0,output_tokens:(.usage.output_tokens // 0)}}
