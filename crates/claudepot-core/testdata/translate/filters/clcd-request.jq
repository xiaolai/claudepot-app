# Extracted from clai clcd (request.jq heredoc). Do not edit; run capture.sh.
def wire($name): ($names[0][$name] // $name);
def selected_model:
  .model as $requested | ($ARGS.named.mapping//{}) as $map |
  if $requested==null then $model
  elif ([$map[]]|index($requested))!=null then $requested
  elif ($requested|IN("opus","fable","sonnet","haiku","subagent")) then $map[$requested]//$model
  elif $requested=="default" then $model
  elif ($requested|startswith("claude-")) then
    ([$requested|capture("(^|-)(?<role>opus|fable|sonnet|haiku)(-|$)")][0].role) as $role | if $role==null then $model else $map[$role]//$model end
  else $requested end;
def parts($role):
  if type == "string" then [{type:(if $role == "assistant" then "output_text" else "input_text" end),text:.}]
  else map(
    if .type == "text" then {type:(if $role == "assistant" then "output_text" else "input_text" end),text:.text}
    elif .type == "image" and $role == "user" then
      {type:"input_image",image_url:(if .source.type == "base64" then "data:"+.source.media_type+";base64,"+.source.data elif .source.type == "url" then .source.url else error("Unsupported image source") end)}
    else error("Unsupported content block: "+(.type // "unknown")) end)
  end;
def message($role;$content): {type:"message",role:$role,content:($content | parts($role))};
def inputs:
  .messages | map((if .role=="system" then "developer" else .role end) as $role |
    if (.content | type) == "string" then [message($role;.content)]
    else reduce .content[] as $b ([];
      if $b.type == "tool_use" then . + [{type:"function_call",call_id:$b.id,name:wire($b.name),arguments:($b.input|tojson)}]
      elif $b.type == "tool_result" then
        . + [{type:"function_call_output",call_id:$b.tool_use_id,output:((if $b.is_error then "[Tool error]\n" else "" end) + (if ($b.content|type)=="string" then $b.content else [($b.content // [])[] | if .type=="text" then .text elif .type=="image" then empty else error("Unsupported tool result block") end] | join("\n") end))}] +
          (if ($b.content|type)=="array" then [$b.content[] | select(.type=="image") | message("user";[.])] else [] end)
      elif $b.type == "thinking" or $b.type == "redacted_thinking" then .
      else . + [message($role;[$b])] end)
    end) | add // [];
. as $original |
.output_config.effort as $requested_effort |
if .temperature!=null or .top_p!=null or .top_k!=null or ((.stop_sequences // [])|length)>0 then
  error("Sampling parameters and stop_sequences are unsupported by this Codex bridge")
elif .output_config.format!=null or .output_format!=null then
  error("Structured output format controls are unsupported by this Codex bridge")
elif .thinking.type=="enabled" then
  error("Fixed thinking budgets are unsupported; use adaptive thinking or CLCD_EFFORT")
elif ($ARGS.named.transport|IN("grok","local","chat")) and $requested_effort!=null then
  error("Reasoning effort is unsupported by this route")
elif $ARGS.named.effort_override!="true" and $requested_effort!=null and (["low","medium","high","xhigh"]|index($requested_effort)|not) then
  error("Unsupported reasoning effort; supported levels: low, medium, high, xhigh")
else . end |
{
  model:selected_model,
  instructions:(if (.system|type)=="string" then .system else [(.system // [])[] | .text // ""] | join("\n") end),
  input:inputs, store:false, stream:true,
  reasoning:{effort:(if $ARGS.named.effort_override=="true" then $effort else .output_config.effort // $effort end)}, parallel_tool_calls:(.tool_choice.disable_parallel_tool_use != true),
  tools:[(.tools // [])[] | if (.name|type)!="string" or .input_schema==null then error("Unsupported server-side tool") else {type:"function",name:wire(.name),description:(.description // ""),parameters:.input_schema,strict:false} end]
} + (if .tool_choice.type == "any" then {tool_choice:"required"}
     elif .tool_choice.type == "none" then {tool_choice:"none"}
     elif .tool_choice.type == "tool" then {tool_choice:{type:"function",name:wire(.tool_choice.name)}} else {} end) |
if $ARGS.named.transport!=null and $ARGS.named.transport!="codex" then
  .+{max_output_tokens:($original.max_tokens // 4096)} |
  if $ARGS.named.transport=="grok" then del(.reasoning) else . end
else . end
