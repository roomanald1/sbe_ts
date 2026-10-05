import { useEffect, useState } from "react";
import type { Observable } from "rxjs";

export function useObservable<T>(
  observable: Observable<T> | undefined,
  initial: T,
  deps: any[]
): T {
  const [value, setValue] = useState<T>(initial);

  useEffect(() => {
    if (!observable) return;

    const sub = observable.subscribe({
      next: setValue,
      error: (err) => console.error("Observable error:", err),
    });

    return () => sub.unsubscribe();
  }, [observable, ...deps]); // only resubscribe if identity changes

  return value;
}
