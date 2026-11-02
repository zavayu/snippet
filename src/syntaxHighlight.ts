export type CodeToken = { text: string; kind?: "keyword" | "string" | "number" | "title" | "comment" | "property" | "meta" };
type Grammar = { name: string; keywords: string; hash?: boolean; slash?: boolean; sql?: boolean; markup?: boolean };
const grammars: Record<string, Grammar> = {
  python: { name: "Python", hash: true, keywords: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield match case" },
  javascript: { name: "JavaScript", slash: true, keywords: "async await break case catch class const continue debugger default delete do else export extends false finally for from function get if import in instanceof let new null of return set static super switch this throw true try typeof undefined var void while with yield" },
  typescript: { name: "TypeScript", slash: true, keywords: "abstract any as async await boolean break case catch class const constructor continue declare default delete do else enum export extends false finally for from function if implements import in infer instanceof interface is keyof let namespace never new null number of private protected public readonly return satisfies static string super switch symbol this throw true try type typeof undefined unknown var void while yield" },
  rust: { name: "Rust", slash: true, keywords: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while" },
  c: { name: "C", slash: true, keywords: "auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while" },
  cpp: { name: "C++", slash: true, keywords: "alignas auto bool break case catch char class concept const constexpr continue default delete do double else enum explicit export extern false float for friend if inline int long namespace new noexcept nullptr operator override private protected public return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile while" },
  java: { name: "Java", slash: true, keywords: "abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for if implements import instanceof int interface long native new null package private protected public record return short static strictfp super switch synchronized this throw throws transient true false try var void volatile while" },
  go: { name: "Go", slash: true, keywords: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var nil true false" },
  csharp: { name: "C#", slash: true, keywords: "abstract as async await base bool break case catch char class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly record ref return sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using var virtual void volatile void while yield" },
  json: { name: "JSON", keywords: "true false null" },
  bash: { name: "Shell", hash: true, keywords: "if then else elif fi for while do done case esac in function select until export local readonly return exit source" },
  sql: { name: "SQL", sql: true, keywords: "select from where insert into values update set delete create alter drop table index view join left right inner outer full on as and or not null true false is in exists like between order by group having limit offset distinct union all with primary key foreign references default case when then else end asc desc count sum avg min max" },
  yaml: { name: "YAML", hash: true, keywords: "true false null yes no" },
  css: { name: "CSS", slash: true, keywords: "important inherit initial unset auto none block flex grid inline relative absolute fixed sticky solid transparent" },
  html: { name: "HTML", markup: true, keywords: "" },
  xml: { name: "XML", markup: true, keywords: "" },
};
const aliases: Record<string, string> = { py: "python", python3: "python", js: "javascript", jsx: "javascript", ts: "typescript", tsx: "typescript", rs: "rust", "c++": "cpp", cc: "cpp", hpp: "cpp", h: "c", cs: "csharp", "c#": "csharp", golang: "go", sh: "bash", shell: "bash", zsh: "bash", yml: "yaml", htm: "html", svg: "xml" };
export function highlightCode(code: string, language: string): { label: string; tokens: CodeToken[] } {
  const key = language.toLowerCase();
  const grammar = grammars[aliases[key] ?? key];
  const label = grammar?.name ?? (language || "Plain text");
  if (!grammar || code.length > 100_000) return { label, tokens: [{ text: code }] };
  const words = new Set(grammar.keywords.split(" "));
  // A single forward pass preserves every character, including unfinished streamed tokens.
  const body = /"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)|"(?:\\[\s\S]|""|[^"\\])*(?:"|$)|'(?:\\[\s\S]|''|[^'\\\n])*(?:'|(?=\n)|$)|`(?:\\[\s\S]|[^`\\])*(?:`|$)|\b(?:0[xX][\da-fA-F]+|0[bB][01]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)(?:[a-zA-Z]\w*)?|[A-Za-z_$][\w$]*|[\s\S]/g;
  const comments = grammar.markup ? /<!--[\s\S]*?(?:-->|$)/.source
    : grammar.slash ? /\/\*[\s\S]*?(?:\*\/|$)|\/\/[^\n]*/.source
    : grammar.hash ? /#[^\n]*/.source : grammar.sql ? /--[^\n]*/.source : "";
  const pattern = new RegExp(comments ? `${comments}|${body.source}` : body.source, "g");
  const tokens: CodeToken[] = [];
  for (const match of code.matchAll(pattern)) {
    const text = match[0], start = match.index, rest = code.slice(start + text.length);
    let kind: CodeToken["kind"];
    const comment = (grammar.markup && text.startsWith("<!--")) || (grammar.slash && (text.startsWith("//") || text.startsWith("/*"))) || (grammar.hash && text.startsWith("#")) || (grammar.sql && text.startsWith("--"));
    if (comment) kind = "comment";
    else if (/^["'`]/.test(text)) kind = /^\s*:/.test(rest) && (key === "json" || key === "yaml" || key === "yml") ? "property" : "string";
    else if (/^\d/.test(text)) kind = "number";
    else if (words.has(grammar.sql ? text.toLowerCase() : text)) kind = "keyword";
    else if (/^[A-Za-z_$]/.test(text)) {
      if (grammar.markup && /<\/?$/.test(code.slice(Math.max(0, start - 2), start))) kind = "keyword";
      else if (/^\s*\(/.test(rest)) kind = "title";
      else if (/^\s*[:=]/.test(rest) && (grammar.markup || key === "css" || key === "yaml" || key === "yml")) kind = "property";
    }
    const last = tokens[tokens.length - 1];
    if (last && last.kind === kind) last.text += text;
    else tokens.push({ text, kind });
  }
  return { label, tokens };
}
