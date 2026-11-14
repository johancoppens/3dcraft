# Hosting 3DCraft for the web

`3dcraft-web-@VERSION@.zip` (from the GitHub release, or `cargo xtask package --formats web`)
holds a static site in `3dcraft-web-@VERSION@/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `@JS@` | wasm-bindgen glue (generated, ES module) |
| `@WASM@` | The app (the engine, the kernel and the viewer), about 16 MB: 6 MB with gzip, 4.5 MB with Brotli |
| `craft3d-worker-<hash>.js` | The worker: Web Workers that run the same module to regenerate and mesh a model and to measure its assertions and outputs off the page's thread (named in `index.html`) |
| `manifest.webmanifest`, `icon-256.png`, `icon-512.png` | So the browser can install it as an app (Chromebooks, Chrome, Edge) |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |
| `LICENSE` | The MIT licence of 3DCraft |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Any path works

All URLs in `index.html` and the manifest are relative, so the site works at a domain root
(`https://example.com/`), under a prefix (`https://example.com/tools/3dcraft/`) and from a CDN
bucket. The `.js` and `.wasm` names carry a content hash of exactly 16 hex digits
(`-[0-9a-f]{16}(_bg)?\.(wasm|js)$`; the worker's too), so they can be cached forever. Only `index.html` and
`manifest.webmanifest` need revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`
  and `.webmanifest` as `application/manifest+json`. Most hosts already do. For nginx, check that
  `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  download from about 16 MB to about 6 MB (gzip) or 4.5 MB (Brotli). You can also precompress (`brotli -k *.wasm`) and let
  the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU only works in a secure context, which means `https://` or
  `http://localhost`. Over plain HTTP elsewhere, the app falls back to WebGL2.
- **No special isolation headers:** 3DCraft runs on the page's thread plus two Web Workers,
  one that regenerates and meshes the model and one for the checks, which talk by messages
  and share no memory. It doesn't use
  `SharedArrayBuffer`, so it doesn't need `Cross-Origin-Opener-Policy` or
  `Cross-Origin-Embedder-Policy`. The worker is a script of the same site: a
  `Content-Security-Policy` must allow it (`worker-src 'self'`, or `script-src 'self'`
  without a `worker-src`). Where a worker cannot be started, the page regenerates the model and
  measures the checks itself, a piece per frame, and is then busier while they run. If your site already sends COEP `require-corp`, also send
  `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin` when the files live on a CDN)
  on the app's files.

nginx example:

```nginx
location /3dcraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; application/manifest+json webmanifest; }
    gzip on;
    gzip_types application/wasm text/javascript text/html application/manifest+json;
    location ~* "-[0-9a-f]{16}(_bg)?\.(wasm|js)$" { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* (index\.html|manifest\.webmanifest)$ { add_header Cache-Control "no-cache"; }
}
```

Serve it over https (or from localhost while testing): browsers let a page use the clipboard only in a
secure context, so elsewhere the status bar's *Copy details* cannot copy and the person copies the
report from its window by hand. WebGPU needs a secure context too.

Local test: `python3 -m http.server 8765 --bind 127.0.0.1` inside the folder, then open
http://localhost:8765/.

## Files and storage

- **Open** (File, Open…, or drop a `.3dcraft` file on the page) and **Import** (a STEP file)
  read the file in the browser; nothing is uploaded anywhere.
- **The system's file dialogs** (the File System Access API: Chrome, Edge, ChromeOS): Open…,
  Import…, Save As… and Export… (STEP, STL, 3MF, OBJ, DXF, SVG) show the operating system's
  file dialog, and Save writes back to the document's file. The API exists only in a **secure
  context** (`https://`, or `http://localhost` / `http://127.0.0.1` while testing) and only in the
  top-level page or a **same-origin** iframe. An OBJ with colours offers its `.mtl` with a button
  of its own, since a browser shows one file dialog per click.
- **Downloads** everywhere else (Firefox, Safari, plain HTTP, or `?download` in the address): Save
  and Export name the file in a dialog of the page and the file is a browser download; Open and
  Import use the browser's file chooser. In a **cross-origin iframe** the browser refuses the
  dialogs: the first refusal switches the app to downloads for the rest of the session and says so
  in the status bar. An OBJ with colours is then two downloads; the browser may ask once to allow
  that.
- There is no crash recovery in the browser: a document lives in the page until it is saved,
  and the browser asks before a page with unsaved changes is closed or reloaded.
- Preferences (open panels, shown sketches and datums, the last export format) are kept in
  `localStorage`. Without storage the app starts with defaults and keeps nothing.
- `?open=models/part.3dcraft` opens a `.3dcraft` or STEP file of the same site at the start
  (relative, or a path from the site's root; other sites are refused).

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/3dcraft/"
  title="3DCraft parametric 3D CAD"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. Preferences are kept in the iframe's `localStorage`. Browsers
  that partition or block third-party storage may forget them between visits, and the app
  then starts with defaults.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without
  `allow-same-origin` there's no storage. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked: the status bar still says *downloaded*, but no file arrives.
- **File dialogs in an iframe:** only a same-origin iframe gets the system's file dialogs; a
  cross-origin one falls back to downloads, which need `allow-downloads` when it is sandboxed.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

## Embedding 3DCraft in a web page (experimental)

*Experimental (2026-11-14): what a page writes may still change between versions.* Instead of an iframe, a
page can load the module itself and drive 3DCraft through the same commands as the control channel: the
page has the controls (sliders, a price, an order button), 3DCraft is the engine and the 3D view. The
protocol, the events and the files are in `docs/control-protocol.md` › *The page as a transport* in the
repository.

**What the page must have.** The app's files beside the page (the module, its glue, the worker, `fonts/`
and `environments/`, fetched relative to the page), and, when the module starts, a canvas with the id
`craft3d_canvas` that asks for the embedded stand with the attribute `data-craft3d-embed`. Its value holds
the options, in the syntax of the address's flags: `presentation=full|view|form`, `warn`, `open=<file of
the same site>`, `webgl`, `cpu`, `download`, `pageregen`, `fullmove`, `movescale=1..4`. An option 3DCraft
does not know or cannot take (`presentation=veiw`, `webgel`) is said in the browser's console and in the
`ready` event's `warnings`, with what is shown instead (for a presentation it does not have, the whole
window). The canvas can name the worker too (`data-craft3d-worker`), so the page needs nothing in its head.
Without the canvas, or without WebGPU and WebGL2, the port says `failed` and answers every request with
that error.

```html
<div style="position: relative; width: 800px; height: 500px">
  <canvas id="craft3d_canvas" data-craft3d-embed="presentation=view"
          data-craft3d-worker="./craft3d-worker-<hash>.js"
          style="position: absolute; inset: 0; width: 100%; height: 100%"></canvas>
</div>
<script type="module">
  import init, { craft3d_connect } from './craft3d-web-<hash>.js';
  await init({ module_or_path: './craft3d-web-<hash>_bg.wasm' });
  const port = craft3d_connect();
  const waiting = new Map();
  let next = 1;
  const call = (method, params, bytes) => new Promise((resolve) => {
    const id = next++;
    waiting.set(id, resolve);
    port.postMessage(bytes ? { id, method, params, bytes } : { id, method, params });
  });
  const ready = new Promise((resolve, reject) => {
    port.onmessage = ({ data }) => {
      if (data.event === 'ready') resolve(data);
      else if (data.event === 'failed') reject(new Error(data.error));
      else if (data.event === 'regenerated') console.log('built', data.revision);
      else { waiting.get(data.id)?.(data); waiting.delete(data.id); }
    };
  });
  await ready;
  const bytes = await (await fetch('shelf.3dcraftjig')).arrayBuffer();
  await call('embed.give', { name: 'shelf.3dcraftjig' }, bytes);
  await call('engine.execute', { command: 'param.set', params: { values: { width: '900 mm' } } });
  const stl = await call('engine.execute', { command: 'file.export', params: { path: 'shelf.stl' } });
  // stl.files: [{ name: 'shelf.stl', bytes: Uint8Array }]
</script>
```

**The presentations**, a setting rather than a build: `full` is the window as the plain page shows it;
`view` is the 3D view with its view cube and controls, without the menu, the tabs, the side panel, the
status bar and the start screen; `form` puts a Jig's parameter form and its outputs beside the view (in a
box narrower than about 700 points, a sheet over the lower part of the view, which then covers part of the
model), without the menu and the tabs. In `view` and `form` the sketches' lines are off, as a visitor wants
the model (`ui.set {"show": {"sketches": true}}` shows them), and back in `full` they are as they were
there. `ui.set {"presentation": "form"}` changes it
while the page runs. Without the whole window a file dropped on the canvas is let be, and the File, Edit
and tab keys are the page's. A document without public parameters shows, in `form`, only that it has none.

**What 3DCraft takes from the page, and what it leaves.** In the embedded stand it leaves the page's title,
its focus (eframe's start gives its own text field the focus before it first waits; 3DCraft gives it back
in the same task, so no key goes astray, though the element that had it hears a `blur` and a `focus`),
the keys outside its canvas (Ctrl+Z and its kin are kept from the browser only while 3DCraft has the
focus), `window.onbeforeunload` (with `warn` it asks before leaving with unsaved changes through a listener
beside the page's own), `localStorage` (no preferences are read or kept) and the page's address (the
options come from the attribute). What any embed of the module keeps: the canvas's `tabindex` and `outline`,
a hidden text field next to the canvas (eframe's, for typing; out of the order Tab follows), listeners on
the canvas and on the document that act only while 3DCraft has the focus or the pointer is over the canvas,
and, while the canvas has the focus, every key but those below. Without the attribute, the page is the
plain app's, exactly as above.

**The keyboard and the wheel in the embedded stand.** While 3DCraft has the keys, a ring in the theme's
accent colour shows round the canvas (eframe takes the canvas's own outline away). eframe keeps Tab inside
its canvas whatever its options say; 3DCraft lets the keyboard out where nothing of its window has the
keys: in `view`, Tab and Shift+Tab move on to the page's next or previous element; in every presentation,
Escape first does what it does in the window, one thing per press (closes a menu, a choice list, a popover
or a dialog, gives a field's keys back, lets go of a joint held by hand, clears the selection while the
pointer is over the view), and the Escape that has nothing left to do leaves the canvas, the same in
Chrome and in Firefox. Every Escape pressed while 3DCraft has the keys is 3DCraft's, the one that leaves
too: a page that wants to know that 3DCraft let go listens to the canvas's `blur`. Where Tab goes after it
is the browser's: Chrome goes on from the canvas, Firefox from the start of the page (Tab to the page's first element, Shift+Tab to
its last). The wheel over the canvas zooms only while 3DCraft has the focus (after a click in it) or with
Ctrl (⌘) held, a pinch on a touchpad; otherwise it scrolls the page. A page that is all configurator and
wants the wheel to zoom without a click first could later ask for it with an option of the attribute, such
as `wheel=zoom`; not built (ask for it in an issue).

**Trust.** The page that loads the module may send every command, with no token and no list of allowed
commands (the developer, 2026-11-14): it runs in the same JavaScript realm as 3DCraft and could call into
the module anyway. So a host page can read everything a person opens in an embedded 3DCraft (a file they
give it, what they change in the form) and change it; embed 3DCraft in a page whose code you trust as you
trust 3DCraft. What a page cannot do through it: reach the person's disk (the files are in memory; Save and
Export go to the page as bytes or to the person as downloads or through their own file dialogs), reach
another site's 3DCraft, or drive the desktop app. A list of allowed commands comes with the iframe's door,
for a page that should not be trusted that far.

**Many requests at once.** A page never holds 3DCraft's thread with its requests, nor has the bodies built
on it: past 64 commands waiting for the worker, and past 8 ms of requests in one frame, the next waits its
turn, in order; at most 1024 wait, and one more is refused with an error that begins `too many requests
wait` (nothing of it was done; send it again once replies have come). A price that follows a slider sends
each change at once and keeps one read outstanding: a read when none waits for its reply, and one more when
that reply comes if the value changed since (`docs/control-protocol.md` › *The page as a transport*).

**Files.** `embed.give` takes a document up to 256 MiB and a STEP file up to 128 MiB. `file.save {path}`
gives the document's file and names the document after it, as Save As does (it counts as saved: with `warn`
nothing is asked before leaving afterwards); `embed.take` gives the document's file and leaves it as it
was. One port for the page's lifetime: a second `craft3d_connect()` throws, also after the page closed its
port, and 3DCraft runs as long as the page (no teardown yet).

## Renderer selection and fallback flags

3DCraft renders with wgpu, with the same shaders as the desktop app. It uses **WebGPU** when the
browser has it and falls back to **WebGL2** on its own. URL query flags override this, and they
work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |
| `?cpu` | Render the viewport with the CPU reference renderer (slowest, most compatible) |
| `?download` | Save and export as browser downloads and open with the file chooser, also where the system's file dialogs exist |
| `?pageregen` | Regenerate and mesh the model on the page's own thread, a piece per frame, also where the worker could (to compare, or when the worker misbehaves) |

For example: `<iframe src="https://example.com/3dcraft/?webgl" ...>`. Flags combine:
`?webgl&open=part.3dcraft`.

When the graphics device reports an error while the app runs, the viewport switches to the CPU
renderer by itself. A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
