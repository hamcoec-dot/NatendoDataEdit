"""Writes the detection test corpus next to this script.

Every file is made up for these tests. Run with any Python 3:
    python make_corpus.py
The expected layout of each file is in expected.txt (read by
tests/detect_corpus.rs). Re-running rewrites the files byte for byte.
"""

import os

HERE = os.path.dirname(os.path.abspath(__file__))
FILES = {}
EXPECTED = []


def add(name, data, sep, quote='"', escape="none", skip=0, sure="any"):
    """`sep=any`: no reading is right (not a table, or one detection cannot
    see); then `sure=no` must hold. `sure=no`: confidence must stay below
    the warning line. `sure=yes`: it must reach it."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    FILES[name] = data
    EXPECTED.append(f"{name} sep={sep} quote={quote} escape={escape} skip={skip} sure={sure}")


def lines(rows, sep=",", end="\n"):
    return end.join(sep.join(r) for r in rows) + end


# --- plain files -------------------------------------------------------------

add("plain_comma.csv", lines([
    ["sku", "item", "bin", "on_hand", "reorder_at"],
    ["K-1001", "hex bolt M6", "A3", "412", "100"],
    ["K-1002", "hex bolt M8", "A3", "96", "100"],
    ["K-1017", "wing nut", "B1", "1500", "250"],
    ["K-1020", "flat washer", "B2", "0", "500"],
    ["K-1033", "spring pin", "C4", "77", "50"],
]), "comma")

add("tab_crlf.tsv", lines([
    ["station", "date", "high_c", "low_c", "rain_mm"],
    ["Elm Ridge", "2025-03-01", "11.4", "2.1", "0.0"],
    ["Elm Ridge", "2025-03-02", "9.8", "3.3", "4.2"],
    ["Fox Hollow", "2025-03-01", "12.0", "1.7", "0.0"],
    ["Fox Hollow", "2025-03-02", "10.1", "2.9", "6.8"],
], sep="\t", end="\r\n"), "tab")

add("semicolon_decimal_comma.csv", lines([
    ["Artikel", "Menge", "Preis", "Summe"],
    ["Schraube", "10", "0,25", "2,50"],
    ["Mutter", "40", "0,10", "4,00"],
    ["Scheibe", "100", "0,05", "5,00"],
    ["Winkel", "3", "1,75", "5,25"],
    ["Dübel", "25", "0,08", "2,00"],
], sep=";"), "semicolon")

add("semicolon_one_decimal_per_row.txt", lines([
    ["Produit", "Prix"],
    ["pomme", "1,20"],
    ["poire", "2,35"],
    ["prune", "0,90"],
    ["pêche", "3,10"],
    ["figue", "4,05"],
], sep=";"), "semicolon")

add("pipe_quoted_commas.psv", lines([
    ["id", "customer", "note"],
    ["1", "\"Okafor, Ada\"", "\"called, left message\""],
    ["2", "\"Lindqvist, Per\"", "paid"],
    ["3", "\"Moreau, Lise\"", "\"wants invoice, PDF\""],
    ["4", "Tan", "\"shipped, tracking sent\""],
], sep="|"), "pipe")

add("comma_quoted_semicolons.csv", lines([
    ["code", "steps", "owner"],
    ["R1", "\"mix; rest; bake\"", "Ines"],
    ["R2", "\"soak; rinse; dry; fold\"", "Ravi"],
    ["R3", "\"heat; stir\"", "Mo"],
    ["R4", "\"chop; fry; serve\"", "Ines"],
]), "comma")

add("quoted_newlines.csv",
    'ticket,summary,detail\n'
    'T-1,printer jam,"Tray 2 jams on every\nthird page.\nCleared twice."\n'
    'T-2,login,"Password reset loop,\nfixed by clearing cache"\n'
    'T-3,monitor,"Flickers at 60 Hz"\n'
    'T-4,keyboard,"Spilled coffee;\nreplaced"\n', "comma")

# --- lines above the table ---------------------------------------------------

add("sep_line.csv", "sep=;\nname;score\nAri;14\nBea;9\nCal;21\n", "semicolon", skip=1)

add("title_preamble.csv",
    "Quarterly parts usage, all depots\n"
    "Printed 2025-04-02\n"
    "\n"
    "depot,part,used,returned\n"
    "North,belt,14,1\n"
    "North,filter,30,0\n"
    "South,belt,9,2\n"
    "South,filter,22,3\n", "comma", skip=3)

add("comment_lines.csv",
    "# export from a field logger\n"
    "# units: metres, seconds\n"
    "run,distance,time\n"
    "1,400,61.2\n"
    "2,400,59.8\n"
    "3,800,131.0\n"
    "4,800,128.4\n", "comma", skip=2)

add("title_with_tabs_count.tsv",
    "Survey results\n"
    "q\tyes\tno\tunsure\n"
    "Q1\t40\t12\t8\n"
    "Q2\t31\t20\t9\n"
    "Q3\t55\t3\t2\n", "tab", skip=1)

# --- shapes ------------------------------------------------------------------

add("single_column.txt", "maple\nbirch\nalder\nrowan\nhazel\nlarch\n", "comma")

add("single_column_decimals.txt", "1.5\n2.25\n3.75\n0.5\n12.0\n", "comma")

add("ragged_rows.csv", lines([
    ["name", "phone", "alt_phone", "email"],
    ["Ines Duarte", "555-0101"],
    ["Ravi Shah", "555-0102", "555-0199", "ravi@example.test"],
    ["Mo Chen", "555-0103", "", "mo@example.test"],
    ["Ade Bello", "555-0104"],
    ["Liv Berg", "555-0105", "555-0177"],
]), "comma")

add("trailing_delimiter.csv",
    "a,b,c,\n1,2,3,\n4,5,6,\n7,8,9,\n", "comma")

add("blank_lines_between.csv",
    "x,y\n\n1,2\n\n3,4\n5,6\n\n\n7,8\n", "comma")

add("two_rows.csv", "city,pop\nLyon,522000\n", "comma")

add("header_only.csv", "alpha;beta;gamma\n", "semicolon")

add("wide_many_columns.csv", lines(
    [[f"m{i}" for i in range(40)]] +
    [[str((r * 7 + c * 3) % 97) for c in range(40)] for r in range(6)]
), "comma")

# --- other separators ---------------------------------------------------------

add("caret.txt", lines([
    ["ID", "CITY", "ZIP"],
    ["7", "Avonlea", "04021"],
    ["8", "Bree", "04022"],
    ["9", "Cair Ford", "04023"],
    ["10", "Dunmore", "04024"],
], sep="^"), "caret")

add("colon.txt", lines([
    ["svc", "x", "1001", "100", "Service Account", "/srv/svc", "/bin/false"],
    ["kim", "x", "1002", "100", "Kim Novak", "/home/kim", "/bin/sh"],
    ["lee", "x", "1003", "100", "Lee Park", "/home/lee", "/bin/sh"],
    ["ops", "x", "1004", "200", "Ops Team", "/home/ops", "/bin/bash"],
], sep=":"), "colon")

add("unit_separator.dat", lines([
    ["key", "label", "value"],
    ["a1", "first, second", "10"],
    ["a2", "third; fourth", "20"],
    ["a3", "fifth|sixth", "30"],
], sep="\x1f"), "us")

add("soh_separated.dat", lines([
    ["8=FIX.4.4", "35=D", "55=ACME"],
    ["8=FIX.4.4", "35=8", "55=ACME"],
    ["8=FIX.4.4", "35=D", "55=BOLT"],
    ["8=FIX.4.4", "35=F", "55=BOLT"],
], sep="\x01"), "soh")

add("double_pipe.txt", lines([
    ["id", "path", "size"],
    ["1", "a|b", "10"],
    ["2", "c", "20"],
    ["3", "d|e|f", "30"],
    ["4", "g", "40"],
], sep="||"), "||")

add("tilde_pipe_tilde.txt", lines([
    ["acct", "holder", "balance"],
    ["001", "Reyes", "1200.50"],
    ["002", "Okoro", "88.00"],
    ["003", "Haas", "10500.75"],
    ["004", "Ito", "0.00"],
], sep="~|~"), "~|~")

add("space_aligned.txt",
    "  id  name          qty    price\n"
    "   1  gear          12     4.50\n"
    "   2  sprocket       3    11.00\n"
    "  10  chain        140     0.85\n"
    "  11  hub            7    23.10\n", "spaces")

add("single_space.txt",
    "x y z\n0.0 1.5 2.0\n1.0 1.4 2.1\n2.0 1.2 2.3\n3.0 0.9 2.6\n", "space")

# --- quoting ----------------------------------------------------------------

add("single_quote.csv", lines([
    ["id", "title", "year"],
    ["1", "'Rain, then sun'", "1998"],
    ["2", "'Night, quietly'", "2004"],
    ["3", "'Glass'", "2011"],
    ["4", "'One, two, three'", "2020"],
]), "comma", quote="'")

add("backslash_escape.csv",
    'id,quote,by\n'
    '1,"She said \\"go, now\\" twice",Ada\n'
    '2,"A \\"plain\\" one",Bo\n'
    '3,"Path C:\\\\tmp, then out",Cy\n'
    '4,"No escapes here",Di\n', "comma", escape="backslash")

add("apostrophes_in_text.csv", lines([
    ["name", "remark", "visits"],
    ["Dara O'Neil", "won't call back", "3"],
    ["Sam D'Souza", "it's fine", "1"],
    ["Kit", "'quoted' word inside", "7"],
    ["Rae O'Hara", "isn't home", "2"],
]), "comma")

add("inch_marks_unquoted.csv", lines([
    ["part", "size", "count"],
    ["pipe", '3/4"', "12"],
    ["pipe", '1"', "30"],
    ["hose", '5/8"', "4"],
    ["hose", '1/2"', "9"],
]), "comma")

add("quoted_everything_semicolon.csv", lines([
    ['"nom"', '"ville"', '"montant"'],
    ['"Durand"', '"Nantes"', '"12,40"'],
    ['"Petit"', '"Lille"', '"7,00"'],
    ['"Roux"', '"Brest"', '"103,99"'],
], sep=";"), "semicolon")

add("json_in_cells.csv",
    'id,payload\n'
    '1,"{""a"": 1, ""b"": [2, 3]}"\n'
    '2,"{""a"": 4, ""b"": [5]}"\n'
    '3,"{""c"": ""x, y""}"\n', "comma")

add("dates_with_commas_quoted.csv", lines([
    ["event", "when", "seats"],
    ["Opening", '"Mar 3, 2025"', "120"],
    ["Talk", '"Mar 4, 2025"', "80"],
    ["Close", '"Mar 9, 2025"', "150"],
]), "comma")

# --- traps --------------------------------------------------------------------

add("dots_in_values.txt", lines([
    ["host", "ip", "version"],
    ["alpha", "10.0.0.11", "2.4.1"],
    ["beta", "10.0.0.12", "2.4.3"],
    ["gamma", "10.0.1.20", "3.0.0"],
    ["delta", "10.0.1.21", "3.1.2"],
], sep=";"), "semicolon")

add("dashes_and_slashes.csv", lines([
    ["when", "phone", "ref"],
    ["2025-01-05", "555-201-3344", "A/17"],
    ["2025-01-06", "555-201-9911", "B/02"],
    ["2025-01-09", "555-830-1200", "A/18"],
    ["2025-01-12", "555-201-7788", "C/01"],
]), "comma")

add("tabs_with_spaces.tsv", lines([
    ["id", "description", "status"],
    ["1", "fix the loose hinge on door two", "open"],
    ["2", "replace the hall light bulb", "done"],
    ["3", "paint the north fence", "open"],
    ["4", "oil the gate", "done"],
], sep="\t"), "tab")

add("tab_with_commas_in_text.txt", lines([
    ["id", "address"],
    ["1", "12 Oak St, Unit 4, Riverton"],
    ["2", "9 Pine Rd, Apt 2, Lakeside"],
    ["3", "40 Elm Ave, Floor 3, Hillcrest"],
], sep="\t"), "tab")

add("pipe_table_edges.md.txt",
    "| name | qty |\n| bolt | 4 |\n| nut | 10 |\n| washer | 25 |\n", "pipe")

# --- encodings ----------------------------------------------------------------

add("utf8_bom_semicolon.csv",
    "\ufeff" + lines([["Name", "Betrag"], ["Ölberg", "3,50"], ["Ärmel", "12,00"], ["Übel", "0,99"]], sep=";"),
    "semicolon")

add("utf16_tab.txt",
    "\ufeff".encode("utf-16-le") + lines([
        ["code", "name", "count"],
        ["1", "Ålesund", "3"],
        ["2", "Évora", "8"],
        ["3", "Łódź", "5"],
    ], sep="\t", end="\r\n").encode("utf-16-le"), "tab")


# --- review round 1: confident wrong answers -------------------------------

add("tiny_numbers.csv", "p,q\n7,8\n", "comma", sure="no")

add("tiny_numbers_three.csv", "u,v\n1,9\n4,2\n", "comma")

add("aligned_blank_cell.txt",
    "item     size  colour\n"
    "lamp     12    amber\n"
    "stool          oak\n"
    "shelf    80    white\n", "any", sure="no")

add("phone_column.txt",
    "phone\n555-201-3344\n555-830-1200\n555-201-7788\n555-664-0912\n", "comma", sure="yes")

add("log_lines.log",
    "2025-02-01 08:15:02 INFO cache warmed\n"
    "2025-02-01 08:15:09 WARN slow query on orders\n"
    "2025-02-01 08:16:40 INFO user signed in\n"
    "2025-02-01 08:17:03 ERROR mail relay refused, retrying\n", "any", sure="no")

add("prose.txt",
    "The barge drifted, slowly, past the mill.\n"
    "Gulls followed it, crying, for a mile.\n"
    "At dusk the lamps came on, one by one.\n", "any", sure="no")

add("title_same_commas.csv",
    "Weekly totals, Ölfjord site\n\nitem,count\nbolts,40\nnuts,22\nwashers,17\n",
    "comma", skip=2)


def main():
    for name, data in FILES.items():
        with open(os.path.join(HERE, name), "wb") as f:
            f.write(data)
    with open(os.path.join(HERE, "expected.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("# file sep quote escape skip (written by make_corpus.py)\n")
        f.write("\n".join(EXPECTED) + "\n")
    print(f"{len(FILES)} files")


if __name__ == "__main__":
    main()
