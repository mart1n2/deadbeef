import wasm from "./pkg/deadbeef_bg.wasm";
import init, { search } from "./pkg/deadbeef.js";

self.onmessage = async (message) => {
  const { safe, prefix, suffix } = message.data;
  try {
    await init({ module_or_path: wasm });
    const creation = search(safe, prefix, suffix);
    self.postMessage({ creation });
  } catch (message) {
    self.postMessage({ creation: null, err: new Error(message) });
  } finally {
    self.close();
  }
};
