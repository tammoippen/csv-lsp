/* csv-lsp documentation — the one shared script: a tiny syntax highlighter.
   Progressive enhancement; the pages read fine as plain text without it.
   One alternation regex per language; the first defined capture group
   decides the color. */
(function () {
  "use strict";
  var esc = function (s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  };
  function paint(src, re, classes) {
    var out = "", last = 0, m;
    re.lastIndex = 0;
    while ((m = re.exec(src)) !== null) {
      var cls = null;
      for (var i = 1; i < m.length; i++) {
        if (m[i] !== undefined) { cls = classes[i - 1]; break; }
      }
      out += esc(src.slice(last, m.index));
      out += cls ? '<span class="hl-' + cls + '">' + esc(m[0]) + "</span>" : esc(m[0]);
      last = m.index + m[0].length;
      if (m.index === re.lastIndex) re.lastIndex++;
    }
    return out + esc(src.slice(last));
  }
  var RUST_KW = "as|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while|async|await";
  var LANGS = {
    rust: {
      re: new RegExp(
        "(\\/\\/[^\\n]*)|(\"(?:[^\"\\\\\\n]|\\\\.)*\")|(#!?\\[[^\\]\\n]*\\])|('[A-Za-z_][A-Za-z0-9_]*(?!'))|\\b([a-z_][A-Za-z0-9_]*!)|\\b(" + RUST_KW + ")\\b|\\b([A-Z][A-Za-z0-9_]*)\\b|\\b(\\d[\\d_]*(?:\\.\\d[\\d_]*)?(?:[iu](?:8|16|32|64|size)|f32|f64)?)\\b",
        "g"),
      classes: ["c", "s", "a", "a", "f", "k", "t", "n"]
    },
    toml: {
      re: /(#[^\n]*)|("(?:[^"\\\n]|\\.)*")|(^\s*\[+[^\]\n]*\]+)|\b(true|false)\b|(^[A-Za-z0-9_.-]+(?=\s*=))/gm,
      classes: ["c", "s", "t", "k", "f"]
    },
    sh: {
      re: /(#[^\n]*)|("(?:[^"\\\n]|\\.)*"|'[^'\n]*')|(\$\{?[A-Za-z_][A-Za-z0-9_]*\}?)/g,
      classes: ["c", "s", "n"]
    },
    json: {
      re: /(\/\/[^\n]*)|("(?:[^"\\]|\\.)*")|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g,
      classes: ["c", "s", "n", "k"]
    },
    tape: {
      re: /(#[^\n]*)|("(?:[^"\\\n]|\\.)*")|^\s*(Output|Require|Set|Type|Enter|Escape|Sleep|Hide|Show|Down|Up|Left|Right|Space|Backspace|Tab|Ctrl\+\S+)\b|\b(\d+(?:\.\d+)?(?:ms|s)?)\b/gm,
      classes: ["c", "s", "k", "n"]
    }
  };
  /* Rainbow columns for csv/tsv/ssv/psv snippets — the same trick the
     tree-sitter grammar plays, including the mini dialect sniff. */
  function paintCsv(src) {
    var counts = { ",": 0, ";": 0, "|": 0, "\t": 0 };
    var lines = src.split("\n");
    var first = "";
    for (var i = 0; i < lines.length; i++) {
      if (lines[i].trim() !== "") { first = lines[i]; break; }
    }
    var inQ = false;
    for (var j = 0; j < first.length; j++) {
      var c = first[j];
      if (c === '"') inQ = !inQ;
      else if (!inQ && counts.hasOwnProperty(c)) counts[c]++;
    }
    var delim = ",", best = 0;
    [",", "\t", ";", "|"].forEach(function (d) {
      if (counts[d] > best) { best = counts[d]; delim = d; }
    });
    return lines.map(function (line) {
      var out = "", col = 0, cell = "", q = false;
      var flush = function () {
        out += '<span class="hl-col' + (col % 7) + '">' + esc(cell) + "</span>";
        cell = "";
      };
      for (var k = 0; k < line.length; k++) {
        var ch = line[k];
        /* Curly quotes guard doc annotations the same way real quotes guard cells. */
        if (ch === '"' || ch === '\u201c' || ch === '\u201d') { q = !q; cell += ch; }
        else if (!q && ch === delim) {
          flush();
          out += '<span class="hl-dim">' + esc(ch) + "</span>";
          col++;
        } else { cell += ch; }
      }
      flush();
      return out;
    }).join("\n");
  }
  try {
    document.querySelectorAll("pre[data-lang]").forEach(function (pre) {
      var lang = pre.getAttribute("data-lang");
      var code = pre.querySelector("code") || pre;
      var src = code.textContent;
      if (lang === "csv" || lang === "tsv" || lang === "ssv" || lang === "psv") {
        code.innerHTML = paintCsv(src);
      } else if (LANGS[lang]) {
        code.innerHTML = paint(src, LANGS[lang].re, LANGS[lang].classes);
      }
    });
  } catch (err) { /* plain text is fine */ }
})();
