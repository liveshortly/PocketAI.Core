// AGT-14: the page behind the hidden web view. Runs model-written code in a
// Worker so the host can kill it: JavaScript in a fresh Blob worker per run,
// Python in one long-lived Pyodide worker (worker-py.js). The Swift side
// calls sandbox.run / sandbox.cancel and also keeps its own deadline.
(function () {
  'use strict';

  // Evaluated inside the Blob worker. Self-contained: no closure over host.js.
  function jsWorkerMain() {
    var post = self.postMessage.bind(self);
    var evalFn = self.eval;
    var encoder = new TextEncoder();
    var decoder = new TextDecoder();
    var cap = 0, bytes = 0, full = false;

    // No network or worker escapes. The CSP is the real wall; this gives the
    // model a clear message instead of a policy error.
    function blocked(name) {
      return function () { throw new Error(name + ' is not available in the sandbox (no network).'); };
    }
    ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'importScripts', 'Worker', 'SharedWorker', 'BroadcastChannel']
      .forEach(function (k) {
        try { Object.defineProperty(self, k, { value: blocked(k), configurable: false, writable: false }); } catch (e) {}
      });

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
      while (cut > 0 && (enc[cut] & 0xC0) === 0x80) cut--;  // stay on a character boundary
      full = true;
      post({ t: 'log', line: decoder.decode(enc.subarray(0, cut)) });
      post({ t: 'trunc' });
    }

    function fmt(v) {
      if (typeof v === 'string') return v.length > cap ? v.slice(0, cap + 1) : v;
      if (v === undefined) return 'undefined';
      if (typeof v === 'function' || typeof v === 'symbol') return String(v);
      if (v instanceof Error) return String(v.stack || v);
      try { var s = JSON.stringify(v); return s === undefined ? String(v) : s; }
      catch (e) { return String(v); }
    }

    function log() { emit(Array.prototype.map.call(arguments, fmt).join(' ')); }
    self.console = { log: log, info: log, debug: log, warn: log, error: log };
    self.print = log;

    function describe(e) {
      return e instanceof Error ? e.name + ': ' + e.message : String(e);
    }

    self.addEventListener('message', function (ev) {
      cap = ev.data.cap;
      Promise.resolve().then(function () {
        return (0, evalFn)(ev.data.code);   // a returned promise is awaited
      }).then(function (v) {
        if (v !== undefined) emit('=> ' + fmt(v));
        post({ t: 'done', error: '' });
      }, function (e) {
        post({ t: 'done', error: describe(e) });
      });
    });
  }
  var jsWorkerSource = '(' + jsWorkerMain.toString() + ')();';

  var current = null;   // { stop(error) } for the run in flight
  var py = null;        // { worker, ready: Promise } while the interpreter is warm

  function seconds(ms) { return Number((ms / 1000).toFixed(1)); }

  // Shared run plumbing: collects lines, enforces the deadline, resolves once.
  function track(worker, timeoutMs, onEnd, send) {
    return new Promise(function (resolve) {
      var lines = [], truncated = false, done = false, t0 = performance.now(), timer = null;
      function finish(error, kill) {
        if (done) return;
        done = true;
        clearTimeout(timer);
        current = null;
        onEnd(kill);
        resolve({
          ok: !error, output: lines.join('\n'), error: error || '',
          durationMs: Math.round(performance.now() - t0), truncated: truncated
        });
      }
      worker.onmessage = function (e) {
        var m = e.data;
        if (m.t === 'log') lines.push(m.line);
        else if (m.t === 'trunc') truncated = true;
        else if (m.t === 'done') finish(m.error, false);
      };
      worker.onerror = function (e) {
        if (e.preventDefault) e.preventDefault();
        finish(e.message || 'The script failed.', true);
      };
      timer = setTimeout(function () { finish('ran too long (' + seconds(timeoutMs) + ' s)', true); }, timeoutMs);
      current = { stop: function (error) { finish(error, true); } };
      try { send(); } catch (err) { finish(String(err), true); }
    });
  }

  function runJS(code, timeoutMs, cap) {
    var url = URL.createObjectURL(new Blob([jsWorkerSource], { type: 'text/javascript' }));
    var worker;
    try { worker = new Worker(url); }
    catch (e) {
      URL.revokeObjectURL(url);
      return Promise.resolve({ ok: false, output: '', error: String(e), durationMs: 0, truncated: false });
    }
    return track(worker, timeoutMs, function () {   // JS: a fresh worker per run
      worker.terminate();   // the kill: stops the script wherever it is, finally blocks included
      URL.revokeObjectURL(url);
    }, function () { worker.postMessage({ code: code, cap: cap }); });
  }

  function dropPython() {
    if (py) { py.worker.terminate(); py = null; }
  }

  function startPython(loadMs) {
    if (py) return py.ready;
    var worker = new Worker('worker-py.js');
    var rec = { worker: worker, ready: null };
    rec.ready = new Promise(function (resolve, reject) {
      var timer = setTimeout(function () { reject(new Error('Python took too long to start.')); }, loadMs);
      worker.onmessage = function (e) {
        if (e.data.t === 'ready') { clearTimeout(timer); resolve(); }
        else if (e.data.t === 'fail') { clearTimeout(timer); reject(new Error(e.data.error)); }
      };
      worker.onerror = function (e) {
        if (e.preventDefault) e.preventDefault();
        clearTimeout(timer);
        reject(new Error(e.message || 'Python could not start.'));
      };
    });
    py = rec;
    return rec.ready;
  }

  async function runPython(code, timeoutMs, cap, loadMs) {
    try {
      await startPython(loadMs);
    } catch (e) {
      dropPython();
      return { ok: false, output: '', error: e.message, durationMs: 0, truncated: false };
    }
    var rec = py;
    return track(rec.worker, timeoutMs, function (kill) {
      if (kill) dropPython();   // a killed interpreter is gone; the next run starts a fresh one
    }, function () { rec.worker.postMessage({ code: code, cap: cap }); });
  }

  window.sandbox = {
    run: function (language, code, timeoutMs, cap, loadMs) {
      if (current) return Promise.resolve({ ok: false, output: '', error: 'Another run is in progress.', durationMs: 0, truncated: false });
      return language === 'python' ? runPython(code, timeoutMs, cap, loadMs) : runJS(code, timeoutMs, cap);
    },
    cancel: function () { if (current) current.stop('Stopped.'); },
    dropPython: dropPython,
    pythonWarm: function () { return py !== null; }
  };
})();
