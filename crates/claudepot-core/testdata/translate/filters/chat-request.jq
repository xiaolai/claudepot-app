# Input is the normalized Responses request; Gemini metadata stays on each call.
def text_parts:
  map(if .type=="input_text" or .type=="output_text" then {type:"text",text:.text}
      elif .type=="input_image" then {type:"image_url",image_url:{url:.image_url}}
      else error("Unsupported chat input part") end);
def call:
  . as $item |
  {id:.call_id,type:"function",function:{name:.name,arguments:.arguments}} +
  (if $provider=="gemini" then
     if $metadata[0][$item.call_id]==null then error("Gemini tool replay metadata is missing; retain the private cache or start a new conversation")
     else {extra_content:($metadata[0][$item.call_id].extra_content // {})} end
   else {} end);
. as $request |
if .parallel_tool_calls==false then error("Disabling parallel tools is unsupported by this chat adapter") else . end |
(reduce .input[] as $item
  ([{role:"system",content:$request.instructions}];
   if $item.type=="message" then .+[{role:(if $item.role=="developer" then "system" else $item.role end),content:($item.content|text_parts)}]
   elif $item.type=="function_call" then
     if .[-1].role=="assistant" then .[-1].tool_calls=((.[-1].tool_calls//[])+[($item|call)])
     else .+[{role:"assistant",content:"",tool_calls:[($item|call)]}] end
   elif $item.type=="function_call_output" then .+[{role:"tool",tool_call_id:$item.call_id,content:$item.output}]
   else error("Unsupported chat input item") end)) as $messages |
{model:.model,messages:$messages,stream:true,stream_options:{include_usage:true},
 tools:[.tools[]|{type:"function",function:{name:.name,description:.description,parameters:.parameters}}],
 max_tokens:(.max_output_tokens // 4096)} +
(if .tool_choice!=null then {tool_choice:(if (.tool_choice|type)=="object" then {type:"function",function:{name:.tool_choice.name}} else .tool_choice end)} else {} end) +
(if $provider=="gemini" then
  if (.reasoning.effort|IN("low","medium","high")) then {reasoning_effort:.reasoning.effort}
  else error("Gemini compatibility supports low, medium or high reasoning effort") end
 else {} end)
