import { useCallback, useRef, useSyncExternalStore } from 'react';
import type { BehaviorSubject } from 'rxjs';

export function useBehaviorSnapshot<T, S = T>(
  observable: BehaviorSubject<T>,
  selector: (value: T) => S = ((value: T) => value as unknown as S),
): S {
  const selectorRef = useRef(selector);
  selectorRef.current = selector;

  const subscribe = useCallback(
    (notify: () => void) => {
      let previous = selectorRef.current(observable.value);
      const subscription = observable.subscribe((value) => {
        const next = selectorRef.current(value);
        if (Object.is(previous, next)) return;
        previous = next;
        notify();
      });
      return () => subscription.unsubscribe();
    },
    [observable],
  );
  const getSnapshot = useCallback(
    () => selectorRef.current(observable.value),
    [observable],
  );

  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
