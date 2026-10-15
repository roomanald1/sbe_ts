import { useSyncExternalStore } from 'react';
import type { BehaviorSubject } from 'rxjs';

export function useBehaviorSnapshot<T>(observable: BehaviorSubject<T>): T {
  return useSyncExternalStore(
    (notify) => {
      const subscription = observable.subscribe(notify);
      return () => subscription.unsubscribe();
    },
    () => observable.value,
    () => observable.value,
  );
}
