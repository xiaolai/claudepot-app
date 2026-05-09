/**
 * Pure-validation tests for citizen-bot lifecycle schemas. No DB
 * dependency — exercises lib/citizen-bots/schemas.ts directly.
 */

import {
  CITIZEN_BOT_CAP_PER_PARENT,
  CITIZEN_BOT_USERNAME_SUFFIX,
  composeCitizenBotUsername,
  createCitizenBotSchema,
  looksLikeCitizenBot,
  mintCitizenBotTokenSchema,
} from "../src/lib/citizen-bots/schemas";
import {
  CITIZEN_SCOPES,
  filterToCitizenScopes,
} from "../src/lib/citizen-bots/scopes";

let pass = 0;
let fail = 0;

function check(name: string, ok: boolean, detail?: string) {
  if (ok) {
    console.log(`PASS  ${name}`);
    pass++;
  } else {
    console.log(`FAIL  ${name}${detail ? "  — " + detail : ""}`);
    fail++;
  }
}

// ── constants ──────────────────────────────────────────────────────
check("cap is 3", CITIZEN_BOT_CAP_PER_PARENT === 3);
check("suffix is @bot", CITIZEN_BOT_USERNAME_SUFFIX === "@bot");
check(
  "compose appends @bot",
  composeCitizenBotUsername("mira") === "mira@bot",
);
check("looksLikeCitizenBot positive", looksLikeCitizenBot("foo@bot"));
check(
  "looksLikeCitizenBot negative for office-bot suffix",
  !looksLikeCitizenBot("mira@reader"),
);
check(
  "looksLikeCitizenBot negative for plain human",
  !looksLikeCitizenBot("xiaolai"),
);

// ── createCitizenBotSchema ────────────────────────────────────────
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "my-helper",
    displayName: "Helper",
    bio: "Does helpful things.",
  });
  check("valid create input passes", r.success);
}
{
  const r = createCitizenBotSchema.safeParse({ baseUsername: "" });
  check("empty username rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "BadCase",
  });
  check("uppercase rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "x".repeat(29),
  });
  check("over-28-chars rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "x".repeat(28),
  });
  check("exactly-28-chars accepted", r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "-leading-dash",
  });
  check("leading-dash rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "trailing-dash-",
  });
  check("trailing-dash rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "ok",
    bio: "x".repeat(281),
  });
  check("over-280-bio rejected", !r.success);
}
{
  const r = createCitizenBotSchema.safeParse({
    baseUsername: "ok",
    extraField: "x",
  });
  check("strict — extra field rejected", !r.success);
}

// ── mintCitizenBotTokenSchema ─────────────────────────────────────
{
  const r = mintCitizenBotTokenSchema.safeParse({
    name: "first-token",
    scopes: ["read:all", "comment:write"],
  });
  check("mint with valid name + scopes passes", r.success);
}
{
  const r = mintCitizenBotTokenSchema.safeParse({ name: "" });
  check("mint with empty name rejected", !r.success);
}
{
  const r = mintCitizenBotTokenSchema.safeParse({ name: "x" });
  check("mint with default empty scopes passes", r.success);
}

// ── filterToCitizenScopes ─────────────────────────────────────────
check(
  "filter passes through allowed scopes",
  filterToCitizenScopes(["read:all", "comment:write"]).length === 2,
);
check(
  "filter drops vote:write",
  !filterToCitizenScopes(["read:all", "vote:write"]).includes(
    "vote:write" as never,
  ),
);
check(
  "filter drops submission:write",
  !filterToCitizenScopes(["submission:write"]).includes(
    "submission:write" as never,
  ),
);
check(
  "filter drops avatar:write",
  !filterToCitizenScopes(["avatar:write"]).includes(
    "avatar:write" as never,
  ),
);
check(
  "filter drops engagement:write",
  !filterToCitizenScopes(["engagement:write"]).includes(
    "engagement:write" as never,
  ),
);
check(
  "filter drops decision:write",
  !filterToCitizenScopes(["decision:write"]).includes(
    "decision:write" as never,
  ),
);
check(
  "filter drops bogus scopes",
  !filterToCitizenScopes(["nonexistent:scope"]).length,
);
check(
  "filter on empty input → empty",
  filterToCitizenScopes([]).length === 0,
);
check(
  "CITIZEN_SCOPES has 5 entries",
  CITIZEN_SCOPES.length === 5,
);
check(
  "CITIZEN_SCOPES does not include vote:write",
  !CITIZEN_SCOPES.includes("vote:write" as never),
);
check(
  "CITIZEN_SCOPES does not include submission:write",
  !CITIZEN_SCOPES.includes("submission:write" as never),
);

console.log(`\n${pass} passed, ${fail} failed`);
if (fail > 0) process.exit(1);
