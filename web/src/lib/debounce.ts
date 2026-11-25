import { useCallback, useEffect, useRef, useState } from "react";

/** The value as it was `ms` after it last changed, for checks that should not run per keystroke. */
export function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

type Settling<T> = {
  timer: ReturnType<typeof setTimeout> | undefined;
  latest: T;
  sent: string;
  save: (value: T) => void;
};

function flush<T>(settling: Settling<T>) {
  clearTimeout(settling.timer);
  settling.timer = undefined;
  const text = JSON.stringify(settling.latest);
  if (text !== settling.sent) {
    settling.sent = text;
    settling.save(settling.latest);
  }
}

/**
 * Saves a value once changes to it pause for `ms`, and once more as the
 * component goes, if it changed since: what `schedule` is given last, unless
 * it is what was saved already. `saved` is what is saved to begin with.
 */
export function useSettledSave<T>(saved: T, onSave: (value: T) => void, ms: number) {
  const settling = useRef<Settling<T>>({
    timer: undefined,
    latest: saved,
    sent: JSON.stringify(saved),
    save: onSave,
  });
  useEffect(() => {
    settling.current.save = onSave;
  }, [onSave]);
  useEffect(() => {
    const current = settling.current;
    return () => flush(current);
  }, []);
  return useCallback(
    (value: T) => {
      const current = settling.current;
      current.latest = value;
      clearTimeout(current.timer);
      current.timer = setTimeout(() => flush(current), ms);
    },
    [ms],
  );
}
