const fs = require("fs");
const path = require("path");

const inPath = path.resolve(__dirname, "../docs/ms-apidoc.md");
const outPath = path.resolve(__dirname, "../docs/ms-apidoc.md");

const METHODS = new Set(["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]);

function extractRawText(md) {
  const start = md.indexOf("```text");
  if (start === -1) return null;
  const afterStart = md.indexOf("\n", start);
  if (afterStart === -1) return null;
  const end = md.indexOf("```", afterStart + 1);
  if (end === -1) return null;
  return md.slice(afterStart + 1, end).replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

function isStatusCode(line) {
  return /^\d{3}$/.test(line.trim());
}

function tryParseJsonBlock(lines, startIndex) {
  // Heuristic: a JSON block starts at a line whose trimmed value starts with { or [
  // and ends when braces/brackets are balanced.
  let i = startIndex;
  const first = (lines[i] ?? "").trim();
  if (!(first.startsWith("{") || first.startsWith("["))) return null;

  let brace = 0;
  let bracket = 0;
  let inString = false;
  let quote = "";
  let escape = false;

  const block = [];
  for (; i < lines.length; i++) {
    const line = lines[i];
    block.push(line);

    for (const ch of line) {
      if (escape) {
        escape = false;
        continue;
      }
      if (inString) {
        if (ch === "\\") {
          escape = true;
          continue;
        }
        if (ch === quote) {
          inString = false;
          quote = "";
        }
        continue;
      }
      if (ch === '"' || ch === "'") {
        inString = true;
        quote = ch;
        continue;
      }
      if (ch === "{") brace++;
      else if (ch === "}") brace--;
      else if (ch === "[") bracket++;
      else if (ch === "]") bracket--;
    }

    if (brace === 0 && bracket === 0) {
      return { block, nextIndex: i + 1 };
    }
  }

  // Unbalanced; treat as not-a-json-block.
  return null;
}

function normalizeLines(raw) {
  return raw
    .split("\n")
    .map((l) => l.replace(/\t/g, "\t").replace(/\s+$/g, ""));
}

function consumeUntil(lines, startIndex, stopPredicate) {
  let i = startIndex;
  const buf = [];
  for (; i < lines.length; i++) {
    if (stopPredicate(lines[i], i)) break;
    buf.push(lines[i]);
  }
  return { buf, nextIndex: i };
}

function parseEndpoints(lines) {
  /** @type {Array<{method:string,path:string,title?:string,description?:string,parameters?:string[],requestBodies?:string[][],responses?:Array<{code:string,lines:string[],json?:string[]}>}>} */
  const endpoints = [];

  let i = 0;
  while (i < lines.length) {
    const t = (lines[i] ?? "").trim();

    if (METHODS.has(t) && i + 1 < lines.length) {
      const p = (lines[i + 1] ?? "").trim();
      if (p.startsWith("/")) {
        const ep = {
          method: t,
          path: p,
          title: undefined,
          description: undefined,
          parameters: [],
          requestBodies: [],
          responses: [],
        };
        i += 2;

        // Title: first non-empty line
        while (i < lines.length && lines[i].trim() === "") i++;
        if (i < lines.length) {
          const maybeTitle = lines[i].trim();
          // Avoid swallowing section keywords
          if (!/^(Parameters|Request body|Responses)$/i.test(maybeTitle) && !METHODS.has(maybeTitle)) {
            ep.title = maybeTitle;
            i++;
          }
        }

        // Description: until Parameters/Request body/Responses/next endpoint
        const desc = [];
        while (i < lines.length) {
          const cur = lines[i];
          const curT = cur.trim();
          if (METHODS.has(curT) && (lines[i + 1] ?? "").trim().startsWith("/")) break;
          if (/^(Parameters|Request body|Responses)$/i.test(curT)) break;
          desc.push(cur);
          i++;
        }
        const descText = desc
          .join("\n")
          .replace(/\n{3,}/g, "\n\n")
          .trim();
        if (descText) ep.description = descText;

        while (i < lines.length) {
          const curT = (lines[i] ?? "").trim();

          if (METHODS.has(curT) && (lines[i + 1] ?? "").trim().startsWith("/")) break;

          if (/^Parameters$/i.test(curT)) {
            i++;
            const { buf, nextIndex } = consumeUntil(lines, i, (line) => {
              const lt = line.trim();
              return (
                /^Request body$/i.test(lt) ||
                /^Responses$/i.test(lt) ||
                (METHODS.has(lt) && (lines[lines.indexOf(line) + 1] ?? "").trim().startsWith("/"))
              );
            });
            const cleaned = buf
              .map((x) => x.trim())
              .filter((x) => x && !/^No parameters$/i.test(x));
            ep.parameters.push(...cleaned);
            i = nextIndex;
            continue;
          }

          if (/^Request body$/i.test(curT)) {
            i++;
            while (i < lines.length && lines[i].trim() === "") i++;
            const jb = tryParseJsonBlock(lines, i);
            if (jb) {
              ep.requestBodies.push(jb.block);
              i = jb.nextIndex;
            }
            continue;
          }

          if (/^Responses$/i.test(curT)) {
            i++;
            // Consume response items until next endpoint or next section.
            while (i < lines.length) {
              const tt = (lines[i] ?? "").trim();
              if (METHODS.has(tt) && (lines[i + 1] ?? "").trim().startsWith("/")) break;
              if (/^(Parameters|Request body)$/i.test(tt)) break;

              if (isStatusCode(tt)) {
                const code = tt;
                i++;
                // Gather lines until next status code / endpoint
                const details = [];
                let json = null;
                while (i < lines.length) {
                  const rt = (lines[i] ?? "").trim();
                  if (isStatusCode(rt)) break;
                  if (METHODS.has(rt) && (lines[i + 1] ?? "").trim().startsWith("/")) break;
                  if (/^(Parameters|Request body|Responses)$/i.test(rt)) break;

                  // Try to capture JSON body
                  if (!json) {
                    const maybe = tryParseJsonBlock(lines, i);
                    if (maybe) {
                      json = maybe.block;
                      i = maybe.nextIndex;
                      continue;
                    }
                  }

                  details.push(lines[i]);
                  i++;
                }

                ep.responses.push({
                  code,
                  lines: details.map((x) => x.trim()).filter(Boolean),
                  json: json ? json : undefined,
                });
                continue;
              }

              // Skip non-informational lines
              i++;
            }
            continue;
          }

          // Skip anything else (tables, links noise)
          i++;
        }

        endpoints.push(ep);
        continue;
      }
    }

    i++;
  }

  return endpoints;
}

function mdEscape(s) {
  return String(s).replace(/\|/g, "\\|");
}

function render(endpoints) {
  const out = [];

  out.push("# Mixing Station API (cleaned)");
  out.push("");
  out.push("This is a cleaned, Markdown-first version of the Mixing Station API export.");
  out.push("It focuses on readable endpoint sections with fenced JSON where available.");
  out.push("");
  out.push("> [!NOTE]\n> This file is generated from the previous export content and intentionally avoids embedding a raw text dump.");
  out.push("");

  // Quick index
  out.push("## Index");
  out.push("");
  out.push("| Method | Path | Title |");
  out.push("|---:|---|---|");
  for (const ep of endpoints) {
    const title = ep.title ? mdEscape(ep.title) : "";
    out.push(`| ${ep.method} | \`${mdEscape(ep.path)}\` | ${title} |`);
  }

  // Group by prefix
  const groups = new Map();
  for (const ep of endpoints) {
    const prefix = ep.path.split("/").slice(1, 3).join("/") || "other"; // e.g. "app/mixers", "console/data"
    if (!groups.has(prefix)) groups.set(prefix, []);
    groups.get(prefix).push(ep);
  }

  const sortedGroupKeys = [...groups.keys()].sort((a, b) => a.localeCompare(b));

  for (const key of sortedGroupKeys) {
    out.push("");
    out.push(`## /${key}`);
    out.push("");

    for (const ep of groups.get(key)) {
      out.push(`### ${ep.method} \`${ep.path}\``);
      if (ep.title) out.push(`**${ep.title}**`);
      out.push("");

      if (ep.description) {
        out.push(ep.description);
        out.push("");
      }

      if (ep.parameters && ep.parameters.length) {
        out.push("#### Parameters");
        out.push("");
        for (const p of ep.parameters) out.push(`- ${p}`);
        out.push("");
      }

      if (ep.requestBodies && ep.requestBodies.length) {
        out.push("#### Request body");
        out.push("");
        for (const rb of ep.requestBodies) {
          out.push("```json");
          out.push(...rb);
          out.push("```");
          out.push("");
        }
      }

      if (ep.responses && ep.responses.length) {
        out.push("#### Responses");
        out.push("");
        for (const r of ep.responses) {
          out.push(`- **${r.code}**${r.lines.length ? ` — ${r.lines[0]}` : ""}`);
          if (r.json) {
            out.push("");
            out.push("  ```json");
            out.push(...r.json.map((l) => "  " + l));
            out.push("  ```");
          }
        }
        out.push("");
      }
    }
  }

  out.push("");
  return out.join("\n");
}

const md = fs.readFileSync(inPath, "utf8");
const raw = extractRawText(md);
if (!raw) {
  console.error("Could not find a ```text fenced block in docs/ms-apidoc.md to convert.");
  process.exit(1);
}

const rawLines = normalizeLines(raw);
const endpoints = parseEndpoints(rawLines);
const rendered = render(endpoints);

fs.writeFileSync(outPath, rendered, "utf8");
console.log(`Wrote cleaned markdown with ${endpoints.length} endpoints to ${outPath}`);
