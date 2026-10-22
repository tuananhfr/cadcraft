# AutoLISP

CADCraft runs AutoLISP routines (`.lsp` files). What is here and what is not yet:
[gaps.md](gaps.md) (E1).

## Loading and running

- **APPLOAD** (Tools > Load Application…, alias `AP`) asks for a `.lsp` file and runs it. Dropping
  a `.lsp` file on the window does the same.
- **`(load "name")`** loads a file by path, with `.lsp` added when the name has none. A file
  APPLOAD loaded is found again by its name, also on the web where there is no file system.
- **`(defun c:NAME …)`** defines a command: type `NAME` at the command line. Built-in commands
  keep their names.
- **At the command line**, `(expression)` evaluates and shows the value, and `!variable` shows a
  variable. Typed at a command's prompt (not a free-text one), an expression's value answers it:
  `CIRCLE 0,0 (* 2 r)`.
- **Programmatically** (scripts, the control channel, MCP, the CLI): `lisp {code, inputs?}` and
  `appload {path} | {name, text | data}` run to the end and return `{value, output, error?}`.
  `inputs` answer `get…` calls in order (numbers, strings, `[x, y]` points); a call with none left
  gets nil.

### At startup

CADCraft loads LISP from its **support folder**, `support` in the settings folder (on Windows
`%APPDATA%\CADCraft\support`, on macOS `~/Library/Application Support/CADCraft/support`, on
Linux `~/.config/cadcraft/support`; created at first start):

- `acad.lsp` once per session, into the first drawing; with `ACADLSPASDOC` set to 1, into every
  drawing;
- `acaddoc.lsp` into every drawing opened or created;
- then `S::STARTUP`, when they define it.

`(autoload "file" '("CMD1" "CMD2"))` in a startup file makes `CMD1` and `CMD2` commands that load
`file` (from the support folder too) the first time they are typed. Startup files run to the end
without asking for input. A drawing's own folder is never searched: an `acaddoc.lsp` that came
with a downloaded drawing does not run.

Each drawing has its own variables and functions. A routine is one undo step, whatever commands
it runs. A routine that waits for input (`getpoint`, `pause`) lets the window work meanwhile;
Esc stops it with "Function cancelled", which `*error*` receives. A long loop runs a slice per
frame, so the window stays live and Esc stops it too.

## Safety

APPLOAD and dropped files ask for review when their UTF-8 source SHA-256 is not approved.
The prompt defaults to No: Yes loads those bytes once, Always also approves the digest,
and No/Enter/Escape leaves the file unloaded. JSON `appload` and `(load)` return
`LISP_NOT_REVIEWED` for unapproved source without prompting. Edited source needs review
again. `lisptrust {action: "list"|"approve"|"revoke", sha256?}` manages approvals;
the LISP panel shows Reviewed/Unreviewed and the first eight digest characters.
Startup files in the support folder retain location-based trust. Direct `lisp {code}`
is available to callers that already have CLI/control authority.

On desktop, `open` write/append and LISP-triggered export/plot/publish outputs must stay
inside canonical allowed directories. Defaults are `<settings>/support` and
`<settings>/lisp-out`; the drawing folder is not automatically allowed.
`lispsandbox {action: "list"|"add"|"remove", path?}` or the panel's **Allowed folders…**
controls change the process-wide roots. Add takes an existing directory path; changes
last for the process. Traversal and symlinks escaping the roots are refused. In the
browser `open` cannot write files; UBPLOT can still deliver PDFs through the host.
SAVE/SAVEAS/WBLOCK through `(command ...)` use host I/O outside this write policy.

Routines with net entity changes produce an audit record containing completion time
(Unix milliseconds), source/digest, created/modified/deleted counts, command ids and
undo label. `lispaudit {limit?: 20}` returns newest first (maximum 1000). The panel shows
the latest 20. Read-only routines have no record, and each routine remains one undo step.
Desktop appends JSONL to `<settings>/logs/lisp-audit.jsonl` with three rotated backups;
the browser keeps only the session ring. This is a local policy, not an OS sandbox or
a tamper-proof audit trail. See [security limits](workspace/security.md).

## The language

Integers are 32-bit and wrap as AutoLISP's do; any real makes arithmetic real. Variables are
dynamically scoped: a function's arguments and its locals (after `/`) are bound while it runs.
Symbols are case-insensitive. Reals print with six significant digits.

Nothing a routine does can crash the program: deep recursion is the error "internal stack limit
reached", malformed source is a reader error, and every built-in checks its arguments.

## Built-in functions

- **Forms:** `quote` `'` `function` `setq` `set` `if` `cond` `while` `repeat` `foreach` `progn`
  `and` `or` `defun` `defun-q` `lambda` `eval` `apply` `mapcar` `load` `autoload` `exit` `quit`
- **Numbers:** `+ - * / rem 1+ 1- abs min max sqrt exp log sin cos atan expt fix float gcd
  logand logior lsh ~ = /= < > <= >= zerop minusp numberp`
- **Lists:** `car cdr` and the `c…r` combinations up to four levels, `cons list append reverse
  length nth last member assoc subst acad_strlsort`, `vl-position vl-remove vl-sort
  vl-remove-if vl-remove-if-not vl-member-if vl-every vl-some vl-list-length vl-consp`
- **Predicates:** `atom listp null not type eq equal boundp vl-symbolp`
- **Strings:** `strcat strlen substr strcase itoa atoi atof chr ascii read wcmatch`,
  `vl-string-search vl-string-subst vl-string-trim vl-string-left-trim vl-string-right-trim
  vl-string-translate vl-string->list vl-list->string vl-prin1-to-string vl-princ-to-string
  vl-symbol-name vl-symbol-value vl-filename-base vl-filename-extension vl-filename-directory`
- **Geometry:** `polar distance angle inters trans` (`trans` returns the point: no UCS yet)
- **Output and files:** `princ prin1 print prompt terpri alert open close read-line write-line
  findfile`
- **Errors:** `*error*`, `vl-catch-all-apply vl-catch-all-error-p vl-catch-all-error-message`
- **Drawing:** `command` (also `command-s`, `vl-cmdf`) with `pause`, a command left open for the
  next call, and `(command)` to cancel; `getvar setvar`; `entget entmake entmod entdel entlast
  entnext entupd handent`; `ssget` (modes X, L, P, I, W, C, F, WP, CP, a point, or the user
  selecting; filters with wildcards and `-4` operators) `ssadd ssdel sslength ssname ssmemb
  sssetfirst ssgetfirst`; `tblsearch tblnext` (LAYER, LTYPE, STYLE, BLOCK, DIMSTYLE, APPID)
- **Block properties:** `(getpropertyvalue ename "EffectiveName"|"BlockName"|"Rotation"|"Position"|"Layer"|"Handle")`.
  Layer/Handle work for any entity; the other properties require an INSERT. Rotation is
  radians and Position is `(x y z)`. Unknown/unavailable properties return an error.
  `(ubplot-attribs ename)` returns ordered `(tag . value)` pairs, or nil for a non-INSERT.
- **Quick Plot:** `(ubplot-scan ss [settings-json])` returns a SheetPlan JSON string;
  the optional JSON object supplies scan parameters. `(ubplot-publish plan-json [path])`
  returns a manifest JSON string and publishes synchronously. Both refuse an active
  command; desktop paths obey the sandbox and `ubplot-publish` requires `.pdf`.
  `(ubplot-templates)` lists approved versioned templates with names, local rectangle
  corners when available, paper and sheet-id tags. General VLA/VLAX and `-plot` prompt
  compatibility remain unavailable; these are native adapters, not ActiveX emulation.
- **Input:** `initget getpoint getcorner getdist getangle getorient getreal getint getstring
  getkword entsel nentsel`
- **Units:** `rtos angtos distof angtof`
- **No-ops:** `vl-load-com gc redraw`; `ver`

Entity lists use the DXF group codes of CADCraft's DXF files: what `entget` returns is what a
DXF file holds for the object, plus `-1` (its name) and `410` (its layout). Angle group
50 is in degrees; `getpropertyvalue` Rotation is in radians.

## Testing against the corpus

The AutoLISP conformance corpus (#648) lives in its own repository. Point `CADCRAFT_LISP_CORPUS`
at it and run `cargo test -p cadcraft-engine --test lisp_corpus`; without the variable the test
says it was skipped.
