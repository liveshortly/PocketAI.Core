// AGT-14: the Pyodide worker. Loads the bundled interpreter (no network: the
// scheme handler serves it from the app bundle) and runs one script per
// message in a fresh namespace. The host terminates this worker on timeout.
/* global loadPyodide, importScripts */
importScripts('pyodide/pyodide.js');

var post = self.postMessage.bind(self);
var encoder = new TextEncoder();
var decoder = new TextDecoder();
var cap = 0, bytes = 0, full = false;

function emit(line) {
  if (full) return;
  var sep = bytes > 0 ? 1 : 0;
  var room = cap - bytes - sep;
  var enc = encoder.encode(line);
  if (enc.length <= room) {
    bytes += enc.length + sep;
    post({ t: 'log', line: line });
    return;
  }
  var cut = Math.max(0, room);
  while (cut > 0 && (enc[cut] & 0xC0) === 0x80) cut--;
  full = true;
  post({ t: 'log', line: decoder.decode(enc.subarray(0, cut)) });
  post({ t: 'trunc' });
}

// Runs `src` in a fresh namespace; returns JSON {error, value}. A traceback
// keeps only the script's own frames.
var HELPER = [
  'import ast, json, sys, traceback',
  'def __pocket_run(src):',
  '    err = value = None',
  '    try:',
  '        tree = ast.parse(src, "<script>", "exec")',
  '        last = None',
  '        if tree.body and isinstance(tree.body[-1], ast.Expr):',
  '            last = ast.Expression(tree.body.pop().value)',
  '        ns = {"__name__": "__main__"}',
  '        exec(compile(tree, "<script>", "exec"), ns)',
  '        if last is not None:',
  '            v = eval(compile(last, "<script>", "eval"), ns)',
  '            if v is not None:',
  '                value = repr(v)',
  '    except BaseException as e:',
  '        tb = e.__traceback__',
  '        if tb is not None:',
  '            tb = tb.tb_next',
  '        err = "".join(traceback.format_exception(type(e), e, tb)).strip()',
  '        lines = err.splitlines()',
  '        if len(lines) > 12:',
  '            err = "\\n".join(lines[:2] + ["  ..."] + lines[-8:])',
  '    finally:',
  '        try:',
  '            sys.stdout.flush(); sys.stderr.flush()',
  '        except Exception:',
  '            pass',
  '    return json.dumps({"error": err, "value": value})'
].join('\n');

var pyodide = null, runner = null;

(async function () {
  try {
    pyodide = await loadPyodide({ indexURL: new URL('pyodide/', self.location.href).href });
    pyodide.runPython(HELPER);
    runner = pyodide.globals.get('__pocket_run');
    post({ t: 'ready' });
  } catch (e) {
    post({ t: 'fail', error: 'Python could not start: ' + (e && e.message || e) });
  }
})();

self.addEventListener('message', function (ev) {
  cap = ev.data.cap; bytes = 0; full = false;
  try {
    pyodide.setStdout({ batched: emit });
    pyodide.setStderr({ batched: emit });
    var res = JSON.parse(runner(ev.data.code));
    if (!res.error && res.value !== null) emit('=> ' + (res.value.length > cap ? res.value.slice(0, cap + 1) : res.value));
    post({ t: 'done', error: res.error || '' });
  } catch (e) {
    post({ t: 'done', error: String(e && e.message || e) });
  }
});
