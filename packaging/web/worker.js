// 3DCraft's worker: the page's own module, measuring the assertions and
// outputs of a document, or regenerating and meshing it, off the page's
// thread (HOSTING.md). The page posts {token, kind, document, work,
// module}; a check is answered with {token, reply}, a scene with a
// {token, part} per part and {token, done}; {token, fatal} when the work
// threw, {token, unavailable} when the module could not be started here.
// A scene is built a piece at a time, and the messages are looked at
// between pieces: a newer scene replaces the one being built and
// {kind: 'cancel'} leaves it, while what was regenerated stays in the
// module for the next state. A command (kind 'command', one that needs
// the bodies) is run between pieces, on what is regenerated, and
// answered with {token, command, files}.
import init, { craft3d_check, craft3d_command, craft3d_scene_begin, craft3d_scene_step } from './@JS@';

// How long a piece of a scene works before the messages are looked at, ms.
const PIECE_MS = 30;

let ready = null;
// The scene being built (its token), and whether the loop runs.
let current = null;
let building = false;

// Between pieces: a message to itself, a task behind the ones waiting
// (a timeout would be clamped to 4 ms).
const channel = new MessageChannel();
let resume = null;
channel.port1.onmessage = () => {
  const go = resume;
  resume = null;
  if (go) go();
};
const pause = () => new Promise((go) => {
  resume = go;
  channel.port2.postMessage(0);
});

async function build() {
  building = true;
  while (current !== null) {
    const token = current;
    try {
      const step = craft3d_scene_step(PIECE_MS);
      if (step === undefined) {
        self.postMessage({ token, done: true });
        if (current === token) current = null;
      } else if (step.length > 0) {
        self.postMessage({ token, part: step }, [step.buffer]);
      }
    } catch (error) {
      self.postMessage({ token, fatal: String(error) });
      if (current === token) current = null;
    }
    await pause();
  }
  building = false;
}

self.onmessage = async (event) => {
  const { token, kind, document, work, module } = event.data;
  if (kind === 'cancel') {
    current = null;
    return;
  }
  try {
    ready = ready || init({ module_or_path: module });
    await ready;
  } catch (error) {
    self.postMessage({ token, unavailable: String(error) });
    return;
  }
  try {
    if (kind === 'scene') {
      craft3d_scene_begin(document, work);
      current = token;
      if (!building) build();
    } else if (kind === 'command') {
      const { answer, files } = craft3d_command(document, work);
      self.postMessage({ token, command: answer, files });
    } else {
      self.postMessage({ token, reply: craft3d_check(document, work) });
    }
  } catch (error) {
    self.postMessage({ token, fatal: String(error) });
  }
};
