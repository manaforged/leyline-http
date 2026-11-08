import re

STRING = {
    "python": r'''(?:[rRbBfFuU]{0,2})(?:"""(?:.|\n)*?"""|\'\'\'(?:.|\n)*?\'\'\'|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*')''',
    "rust": r'''(?:b?r#*"(?:.|\n)*?"#*|b?"(?:\\.|[^"\\])*"|b?'(?:\\.|[^'\\])'|'[A-Za-z_]\w*)''',
    "node": r'''(?:`(?:\\.|[^`\\])*`|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*')''',
}
COMMENT = {
    "python": r"\#[^\n]*",
    "rust": r"//[^\n]*|/\*(?:.|\n)*?\*/",
    "node": r"//[^\n]*|/\*(?:.|\n)*?\*/",
}
OPERATORS = (
    r"\.\.\.|\*\*=|===|!==|>>=|<<=|=>|->|::|==|!=|<=|>=|&&|\|\||\+=|-=|\*=|/=|\*\*|//|\?\.|\?\?|\.\.|\+\+|--|:=|<<|>>"
)
NUMBER = r"\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?(?:[a-z]\w*)?"
IDENT = r"[^\W\d]\w*|\$\w*"


def compile_lexer(lang):
    return re.compile(
        rf"(?P<comment>{COMMENT[lang]})|(?P<string>{STRING[lang]})|(?P<number>{NUMBER})"
        rf"|(?P<ident>{IDENT})|(?P<op>{OPERATORS})|(?P<punct>[^\s])"
    )


def tokens(source, lang):
    out = []
    for match in compile_lexer(lang).finditer(source):
        kind = match.lastgroup
        if kind != "comment":
            out.append((kind, match.group()))
    return out


def code_lines(source, lang):
    lines = set()
    for match in compile_lexer(lang).finditer(source):
        if match.lastgroup == "comment":
            continue
        start = source.count("\n", 0, match.start())
        end = start + match.group().count("\n")
        lines.update(range(start, end + 1))
    return len(lines)
