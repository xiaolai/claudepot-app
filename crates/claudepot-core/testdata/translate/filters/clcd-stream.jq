# Extracted from clai clcd (stream.jq heredoc). Do not edit; run capture.sh.
def event($type;$body): {type:$type} + $body;
def emit: .events[] | "event: \(.type)\ndata: \(tojson)\n\n";
def start($key;$block):
  .blocks[$key]={index:.next,open:true} | .next+=1 |
  .events += [event("content_block_start";{index:.blocks[$key].index,content_block:$block})];
def stop($key):
  if .blocks[$key].open then .blocks[$key].open=false |
    .events += [event("content_block_stop";{index:.blocks[$key].index})] else . end;
foreach (inputs, "__clcd_eof__") as $rawline
  ({blocks:{},next:0,tool:false,ended:false,events:[]};
   ($rawline|rtrimstr("\r")) as $line | .events=[] |
   if $line=="__clcd_eof__" then
     if .ended then . else error("Codex stream ended before completion") end
   elif ($line|startswith("data: ")) and $line!="data: [DONE]" then
     ($line[6:] | fromjson) as $e |
     ($e.item_id // $e.item.id // "") as $id |
     if $e.type=="response.created" then
       .events=[event("message_start";{message:{id:($e.response.id // "msg_clcd"),type:"message",role:"assistant",model:$model,content:[],stop_reason:null,stop_sequence:null,usage:{input_tokens:0,output_tokens:0,cache_creation_input_tokens:0,cache_read_input_tokens:0}}})]
     elif $e.type=="response.output_item.added" and $e.item.type=="function_call" then
       .tool=true | start($id;{type:"tool_use",id:$e.item.call_id,name:($names[0][$e.item.name] // $e.item.name),input:{}})
     elif $e.type=="response.content_part.added" and ($e.part.type|IN("output_text","refusal")) then
       start($id+":"+($e.content_index|tostring);{type:"text",text:""})
     elif $e.type=="response.output_text.delta" or $e.type=="response.refusal.delta" then
       ($id+":"+($e.content_index|tostring)) as $k |
       (if .blocks[$k]==null then start($k;{type:"text",text:""}) else . end) |
       .events += [event("content_block_delta";{index:.blocks[$k].index,delta:{type:"text_delta",text:$e.delta}})]
     elif $e.type=="response.function_call_arguments.delta" then
       .blocks[$id].arguments=true |
       .events += [event("content_block_delta";{index:.blocks[$id].index,delta:{type:"input_json_delta",partial_json:$e.delta}})]
     elif $e.type=="response.output_item.done" and $e.item.type=="function_call" then
       (if .blocks[$id].arguments then . else .events += [event("content_block_delta";{index:.blocks[$id].index,delta:{type:"input_json_delta",partial_json:($e.item.arguments // "{}")}})] end) | stop($id)
     elif $e.type=="response.content_part.done" and ($e.part.type|IN("output_text","refusal")) then stop($id+":"+($e.content_index|tostring))
     elif $e.type=="response.completed" or $e.type=="response.incomplete" then
       reduce (.blocks|keys[]) as $k (.;stop($k)) |
       .ended=true | ($e.response.usage // {}) as $u |
       .events += [event("message_delta";{delta:{stop_reason:(if $e.type=="response.incomplete" then "max_tokens" elif .tool then "tool_use" else "end_turn" end),stop_sequence:null},usage:{input_tokens:(($u.input_tokens // 0)-($u.input_tokens_details.cached_tokens // 0)),cache_read_input_tokens:($u.input_tokens_details.cached_tokens // 0),output_tokens:($u.output_tokens // 0)}}),event("message_stop";{})]
     elif $e.type=="response.failed" or $e.type=="error" then
       .ended=true | .events=[event("error";{error:{type:"api_error",message:"Codex returned an error."}})]
     else . end
   else . end;
   emit)
