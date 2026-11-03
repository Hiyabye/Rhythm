// Compatibility entry point: the explicit GPU validator always targets NVIDIA Vulkan.
import { main } from './validate-worker.mjs';

await main(['--backend', 'vulkan', ...process.argv.slice(2)]);
