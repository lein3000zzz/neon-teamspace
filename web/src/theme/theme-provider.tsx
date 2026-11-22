import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import {
  applyTheme,
  browserStorage,
  clearPreferences,
  DARK_QUERY,
  DEFAULT_PREFERENCES,
  loadPreferences,
  resolveMode,
  savePreferences,
  STORAGE_KEY,
  type Mode,
  type ThemePreferences,
} from "./preferences";

interface ThemeContextValue {
  preferences: ThemePreferences;
  /** The mode actually shown, with "system" resolved. */
  mode: Mode;
  /** The device's colour scheme, whatever the user picked. */
  systemMode: Mode;
  update: (patch: Partial<ThemePreferences>) => void;
  /** Back to the defaults, and forget the stored choice. */
  reset: () => void;
  /**
   * Takes preferences kept elsewhere, the account's, and caches them on this
   * device for the next first paint. The defaults are cached as no choice.
   */
  load: (preferences: ThemePreferences) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function subscribeToSystemMode(onChange: () => void) {
  const query = window.matchMedia?.(DARK_QUERY);
  query?.addEventListener("change", onChange);
  return () => query?.removeEventListener("change", onChange);
}

function systemPrefersDark() {
  return window.matchMedia?.(DARK_QUERY).matches ?? false;
}

/**
 * Owns the theme once React is running: keeps <html> in sync with the
 * preferences, follows the OS colour scheme live in system mode, and picks up
 * changes made in other tabs.
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preferences, setPreferences] = useState(() => loadPreferences(browserStorage()));
  // Only the user's own changes, and the account's choice, are written back.
  // Values loaded from storage are not, so someone who never chose keeps
  // following the defaults.
  const pendingWrite = useRef<"save" | "clear" | null>(null);
  const prefersDark = useSyncExternalStore(subscribeToSystemMode, systemPrefersDark);
  const mode = resolveMode(preferences.mode, prefersDark);
  const systemMode = resolveMode("system", prefersDark);

  useEffect(() => {
    applyTheme(document.documentElement, preferences, mode);
  }, [preferences, mode]);

  useEffect(() => {
    const write = pendingWrite.current;
    pendingWrite.current = null;
    if (write === "save") {
      savePreferences(browserStorage(), preferences);
    } else if (write === "clear") {
      clearPreferences(browserStorage());
    }
  }, [preferences]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY || event.key === null) {
        setPreferences(loadPreferences(browserStorage()));
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const update = useCallback((patch: Partial<ThemePreferences>) => {
    pendingWrite.current = "save";
    setPreferences((current) => ({ ...current, ...patch }));
  }, []);

  const reset = useCallback(() => {
    pendingWrite.current = "clear";
    setPreferences({ ...DEFAULT_PREFERENCES });
  }, []);

  const load = useCallback((next: ThemePreferences) => {
    pendingWrite.current = isDefault(next) ? "clear" : "save";
    setPreferences({ ...next });
  }, []);

  const value = useMemo(
    () => ({ preferences, mode, systemMode, update, reset, load }),
    [preferences, mode, systemMode, update, reset, load],
  );

  return <ThemeContext value={value}>{children}</ThemeContext>;
}

function isDefault(preferences: ThemePreferences): boolean {
  return (Object.keys(DEFAULT_PREFERENCES) as (keyof ThemePreferences)[]).every(
    (key) => preferences[key] === DEFAULT_PREFERENCES[key],
  );
}

export function useTheme(): ThemeContextValue {
  const context = use(ThemeContext);
  if (!context) {
    throw new Error("useTheme must be used inside <ThemeProvider>");
  }
  return context;
}
