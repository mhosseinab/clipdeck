const KEYWORDS =
  "const|let|var|function|class|if|else|for|while|return|import|from|export|new|this|def|self|None|True|False|async|await|try|catch|except|throw|typeof|instanceof|in|of|switch|case|break|continue|do|void|delete|yield|static|public|private|protected|extends|implements|interface|enum|struct|fn|mut|use|pub|impl|match|loop|package|type";

const TOKEN = new RegExp(
  `(\\/\\/[^\\n]*|#[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/)|("(?:\\\\.|[^"\\\\])*"|'(?:\\\\.|[^'\\\\])*'|\`(?:\\\\.|[^\`\\\\])*\`)|\\b(${KEYWORDS})\\b|\\b(0x[0-9a-fA-F]+|\\d+\\.?\\d*)\\b`,
  "g",
);

function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Minimal dependency-free syntax highlighting for code previews. */
export function highlight(src: string): string {
  let out = "";
  let last = 0;
  for (const m of src.matchAll(TOKEN)) {
    const i = m.index ?? 0;
    out += esc(src.slice(last, i));
    const text = esc(m[0]);
    if (m[1]) out += `<span class="tok-comment">${text}</span>`;
    else if (m[2]) out += `<span class="tok-string">${text}</span>`;
    else if (m[3]) out += `<span class="tok-keyword">${text}</span>`;
    else out += `<span class="tok-number">${text}</span>`;
    last = i + m[0].length;
  }
  out += esc(src.slice(last));
  return out;
}
