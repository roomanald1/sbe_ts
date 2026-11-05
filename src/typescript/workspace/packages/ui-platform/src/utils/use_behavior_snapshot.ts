import { useCallback, useRef, useSyncExternalStore } from 'react';
import type { BehaviorSubject } from 'rxjs';

export function useBehaviorSnapshot<T, S = T>(
  observable: BehaviorSubject<T>,
  selector: (value: T) => S = ((value: T) => value as unknown as S),
): S {
  const selectorRef = useRef(selector);
  selectorRef.current = selector;
  const snapshotRef = useRef<{ source: BehaviorSubject<T>; value: S }>({
    source: observable,
    value: selector(observable.value),
  });

  // useSyncExternalStore requires getSnapshot to return the same reference
  // until the store changes. Keep the selected value cached instead of
  // recomputing it during React's render checks.
  if (snapshotRef.current.source !== observable) {
    snapshotRef.current = { source: observable, value: selector(observable.value) };
  }

  const subscribe = useCallback(
    (notify: () => void) => {
      let isInitialValue = true;
      const subscription = observable.subscribe((value) => {
        if (isInitialValue) {
          isInitialValue = false;
          snapshotRef.current = {
            source: observable,
            value: selectorRef.current(value),
          };
          return;
        }
        const next = selectorRef.current(value);
        if (Object.is(snapshotRef.current.value, next)) return;
        snapshotRef.current = { source: observable, value: next };
        notify();
      });
      return () => subscription.unsubscribe();
    },
    [observable],
  );
  const getSnapshot = useCallback(() => snapshotRef.current.value, []);

  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
