// 3DCraft's checking worker: the page's own module, measuring the
// assertions and outputs of a document off the page's thread
// (HOSTING.md). The page posts {token, document, work, module}; the
// answer is {token, reply}, {token, fatal} when measuring threw, or
// {token, unavailable} when the module could not be started here.
import init, { craft3d_check } from './@JS@';

let ready = null;
self.onmessage = async (event) => {
  const { token, document, work, module } = event.data;
  try {
    ready = ready || init({ module_or_path: module });
    await ready;
  } catch (error) {
    self.postMessage({ token, unavailable: String(error) });
    return;
  }
  try {
    self.postMessage({ token, reply: craft3d_check(document, work) });
  } catch (error) {
    self.postMessage({ token, fatal: String(error) });
  }
};
