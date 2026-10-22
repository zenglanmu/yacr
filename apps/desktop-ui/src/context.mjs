// Shared React contexts. Regions read the translator and the shell actions
// from here instead of threading a dozen props through every level, so a new
// region only imports `useI18n()` / `useShell()`.
import { createContext, useContext } from 'react';

export const I18nContext = createContext(null);
export const ShellContext = createContext(null);

export function useI18n() {
  const value = useContext(I18nContext);
  if (!value) throw new Error('useI18n must be used inside <I18nContext.Provider>');
  return value;
}

export function useShell() {
  const value = useContext(ShellContext);
  if (!value) throw new Error('useShell must be used inside <ShellContext.Provider>');
  return value;
}
