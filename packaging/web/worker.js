// 3DCraft's worker: the page's own module, measuring the assertions and
// outputs of a document, or regenerating and meshing it, off the page's
// thread (HOSTING.md). The page posts {token, kind, document, work,
// module}; a check is answered with {token, reply}, a scene with a
// {token, part} per part and {token, done}; {token, fatal} when the work
// threw, {token, unavailable} when the module could not be started here.
import init, { craft3d_check, craft3d_scene_begin, craft3d_scene_next } from './@JS@';

let ready = null;
self.onmessage = async (event) => {
  const { token, kind, document, work, module } = event.data;
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
      for (let part = craft3d_scene_next(); part !== undefined; part = craft3d_scene_next()) {
        self.postMessage({ token, part }, [part.buffer]);
      }
      self.postMessage({ token, done: true });
    } else {
      self.postMessage({ token, reply: craft3d_check(document, work) });
    }
  } catch (error) {
    self.postMessage({ token, fatal: String(error) });
  }
};
