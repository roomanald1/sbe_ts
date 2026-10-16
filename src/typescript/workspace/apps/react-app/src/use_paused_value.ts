import { useLayoutEffect, useRef } from 'react';

/** Keep grid rows stable during a short dashboard drag interaction. */
export function usePausedValue<T>(value: T, paused: boolean): T {
  const latest = useRef(value);
  useLayoutEffect(() => {
    if (!paused) latest.current = value;
  }, [value, paused]);
  return paused ? latest.current : value;
}
