// The theme toggle: one icon button, shared by the desktop title row and the
// compact top bar so both layouts offer the same switch.
//
// The button never flips a local theme. It writes the opposite of the **host's**
// value through `cad_set_setting`; the icon changes only when the next
// `cad_status.theme` reports the new value. A refused write keeps the current
// theme and says so in the tooltip (and to a screen reader) with the Options
// dialog's own "settings change refused" sentence, because a silently dead
// toggle is the exact defect this path exists to avoid.
import React, { useState } from 'react';
import { IconButton } from './primitives.tsx';
import type { ButtonSize } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { oppositeTheme, setTheme, useHostTheme } from '../theme/theme.ts';
import { reportedSurfaceMode } from '../cad/surface.ts';
import type { Theme } from '../theme/theme.ts';

export interface ThemeToggleProps {
  /** Button size; the compact top bar overrides it to a 44px touch target in CSS. */
  readonly size?: ButtonSize;
}

/** The one theme switch. */
export function ThemeToggle({ size = 'sm' }: ThemeToggleProps) {
  const { t } = useI18n();
  const shell = useShell();
  const theme = useHostTheme(shell.status?.theme, reportedSurfaceMode(shell));
  const target = oppositeTheme(theme);
  const [failure, setFailure] = useState('');
  const themeName = (value: Theme) => t(value === 'light' ? 'shell.theme.light' : 'shell.theme.dark');
  const label = t('shell.theme.toggle', { theme: themeName(target) });
  const onClick = () => {
    setFailure('');
    void setTheme(target).then(result => {
      if (result.kind === 'failed') {
        setFailure(result.code ? `${t('options.error.generic')} (${result.code})` : t('options.error.generic'));
      }
    });
  };
  return (
    <>
      <IconButton
        icon={target === 'light' ? 'sun' : 'moon'}
        label={label}
        title={failure || label}
        size={size}
        onClick={onClick}
      />
      {failure ? <span className="sr-only" role="alert">{failure}</span> : null}
    </>
  );
}
