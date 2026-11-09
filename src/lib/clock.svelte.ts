import type { ClockFormat } from './types';

// The 12/24-hour preference from Settings. App keeps it in step with the snapshot;
// components pass `clock.format` to the formatters in archive-format.ts.
export const clock = $state<{ format: ClockFormat }>({ format: '12h' });
