def response($type;$body): {kind:"response",value:({type:$type}+$body)};
def complete:
  if .finished|not then error("Chat stream ended before a finish reason") else . end |
  if .ended then . else .ended=true |
    .events += [response((if .finish=="length" then "response.incomplete" else "response.completed" end);
      {response:{id:.id,status:(if .finish=="length" then "incomplete" else "completed" end),
       output:([(if .text then {type:"message",content:[{type:"output_text",text:.text_content}]} else empty end)] +
         [.calls|to_entries|sort_by(.key|tonumber)[]|.value|{type:"function_call",call_id:.id,name:.name,arguments:.arguments}]),
       usage:{input_tokens:(.usage.prompt_tokens//0),output_tokens:(.usage.completion_tokens//0),input_tokens_details:{cached_tokens:(.usage.prompt_tokens_details.cached_tokens//0)}}}})] end;
foreach (inputs,"__clai_eof__") as $rawline
  ({started:false,text:false,text_content:"",calls:{},finished:false,ended:false,events:[],usage:{},id:"resp_chat"};
   ($rawline|rtrimstr("\r")) as $line | .events=[] |
   if $line=="data: [DONE]" or $line=="__clai_eof__" then complete
   elif ($line|startswith("data: ")) then
     ($line[6:]|fromjson) as $e |
     if $e.error!=null then error("Chat upstream returned an error") else . end |
     (if $e.usage!=null then .usage=$e.usage else . end) |
     (if .started then . else .started=true | .id=($e.id//.id) |
       .events += [response("response.created";{response:{id:.id}})] end) |
     ($e.choices[0] // {}) as $choice | ($choice.delta // {}) as $delta |
     (if ($delta.content//"")!="" then
       (if .text then . else .text=true | .events += [response("response.content_part.added";{item_id:"chat_text",content_index:0,part:{type:"output_text",text:""}})] end) |
       .text_content += $delta.content |
       .events += [response("response.output_text.delta";{item_id:"chat_text",content_index:0,delta:$delta.content})]
      else . end) |
     reduce (($delta.tool_calls // [])|to_entries[]) as $entry (.;
       $entry.value as $call |
       ($call.index // (if $call.id!=null then ([.calls|to_entries[]|select(.value.id==$call.id)|.key|tonumber][0] // (.calls|length)) elif (.calls|length)==1 then 0 else error("Parallel chat tool fragment has neither index nor id") end)) as $index |
       ($index|tostring) as $key |
       .calls[$key]=((.calls[$key] // {id:"",name:"",arguments:"",extra_content:{}}) |
         .id=($call.id//.id) | .name=($call.function.name//.name) |
         .arguments += ($call.function.arguments//"") |
         .extra_content *= ($call.extra_content // {}))) |
     if $choice.finish_reason!=null and (.finished|not) then
       .finished=true | .finish=$choice.finish_reason |
       if .finish=="content_filter" then error("Chat response was blocked by content filtering") else . end |
       (if .text then .events += [response("response.content_part.done";{item_id:"chat_text",content_index:0,part:{type:"output_text"}})] else . end) |
       reduce (.calls|to_entries|sort_by(.key|tonumber)[]) as $entry (.;
         $entry.value as $call |
         if $call.id=="" or $call.name=="" or (($call.arguments|fromjson|type)!="object") then error("Incomplete chat tool call") else . end |
         .events += [{kind:"metadata",value:{id:$call.id,model:$model,extra_content:$call.extra_content}},
           response("response.output_item.added";{item:{id:$call.id,type:"function_call",call_id:$call.id,name:$call.name}}),
           response("response.output_item.done";{item:{id:$call.id,type:"function_call",call_id:$call.id,name:$call.name,arguments:$call.arguments}})])
     else . end
   else . end;
   .events[] | if .kind=="metadata" then "cache: \(.value|tojson)\n" else "data: \(.value|tojson)\n\n" end)
