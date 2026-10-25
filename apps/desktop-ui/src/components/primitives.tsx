// Reusable interactive primitives. Everything the chrome does is built from
// these pieces plus <Icon>, so hover/pressed/disabled/focus states stay identical
// across regions.
import React from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';

/** The surface treatment of a text button. */
export type ButtonVariant = 'ghost' | 'solid' | 'chip' | 'plain';

/** The control height class of a button. */
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg';

/** A text button: an optional leading icon, a label and an optional trailing slot. */
export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** Leading glyph, drawn at `iconSize` (16px by default). */
  readonly icon?: IconName;
  readonly iconSize?: number;
  readonly variant?: ButtonVariant;
  readonly size?: ButtonSize;
  /** Maps to `aria-pressed` and the ON style; a boolean makes it a toggle. */
  readonly pressed?: boolean;
  readonly active?: boolean;
  readonly trailing?: React.ReactNode;
  /**
   * The `<button>` node itself. React 19 passes `ref` as an ordinary prop, so it
   * travels through the rest spread to the element; it is declared here because
   * `ButtonHTMLAttributes` does not carry it. A caller needs it when focus has to
   * come *back* to the button that opened something (the layers palette hands it
   * back after an inline form closes).
   */
  readonly ref?: React.Ref<HTMLButtonElement>;
}

/**
 * A text button. `variant` picks the surface treatment: `ghost` (ribbon/tab),
 * `solid` (primary action), `chip` (status-bar toggle), `plain` (menus).
 * `pressed` maps to `aria-pressed` and the ON style for toggles.
 */
export function Button({
  children,
  icon,
  iconSize = 16,
  variant = 'ghost',
  size = 'md',
  pressed,
  active,
  disabled,
  title,
  trailing,
  onClick,
  className,
  type = 'button',
  ...rest
}: ButtonProps) {
  const isToggle = typeof pressed === 'boolean';
  return (
    <button
      type={type}
      className={[
        'ui-button',
        `ui-button--${variant}`,
        `ui-button--${size}`,
        active ? 'is-active' : '',
        isToggle && pressed ? 'is-on' : '',
        className,
      ].filter(Boolean).join(' ')}
      disabled={disabled}
      title={title}
      aria-pressed={isToggle ? pressed : undefined}
      onClick={disabled ? undefined : onClick}
      {...rest}
    >
      {icon ? <Icon name={icon} size={iconSize} /> : null}
      {children != null && children !== false ? <span className="ui-button__label">{children}</span> : null}
      {trailing ? <span className="ui-button__trailing">{trailing}</span> : null}
    </button>
  );
}

/** A square icon-only button: the label is required (it is the accessible name). */
export interface IconButtonProps extends Omit<ButtonProps, 'icon' | 'title' | 'children'> {
  /** The glyph to draw. */
  readonly icon: IconName;
  /** The accessible name; also the default tooltip. */
  readonly label: string;
  /** Optional tooltip that overrides `label` (the accessible name stays `label`). */
  readonly title?: string;
  readonly children?: never;
}

/** A square icon-only button; `title` is required for accessibility. */
export function IconButton({ icon, label, iconSize = 18, size = 'md', variant = 'ghost', ...rest }: IconButtonProps) {
  return (
    <Button
      variant={variant}
      size={size}
      className="ui-icon-button"
      title={label}
      aria-label={label}
      icon={icon}
      iconSize={iconSize}
      {...rest}
    />
  );
}

/** A ribbon command button: a large glyph over a label. */
export interface RibbonButtonProps {
  readonly icon: IconName;
  readonly label: string;
  /** The refusal sentence shown while the button is disabled. */
  readonly hint?: string;
  readonly disabled?: boolean;
  readonly active?: boolean;
  readonly onClick?: () => void;
}

/** A ribbon command button: large icon over a label, AutoCAD-style. */
export function RibbonButton({ icon, label, hint, disabled, active, onClick }: RibbonButtonProps) {
  return (
    <button
      type="button"
      className={['ribbon-button', active ? 'is-active' : '', disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      disabled={disabled}
      title={disabled ? hint : undefined}
      aria-disabled={disabled || undefined}
      onClick={disabled ? undefined : onClick}
    >
      <Icon name={icon} size={26} />
      <span className="ribbon-button__label">{label}</span>
    </button>
  );
}

/** A status-bar chip: the one surface every toggle uses (drafting aids and flag parameters). */
export interface ToggleProps {
  readonly label: React.ReactNode;
  readonly on?: boolean;
  readonly onClick?: () => void;
  readonly title?: string;
  readonly disabled?: boolean;
}

/** A status-bar chip. `on` maps to `aria-pressed` and the ON style. */
export function Toggle({ label, on, onClick, title, disabled }: ToggleProps) {
  return (
    <button
      type="button"
      className={['status-toggle', on ? 'is-on' : '', disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      aria-pressed={Boolean(on)}
      disabled={disabled}
      title={title}
      onClick={disabled ? undefined : onClick}
    >
      {label}
    </button>
  );
}
