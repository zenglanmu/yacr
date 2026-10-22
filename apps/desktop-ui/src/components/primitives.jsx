// Reusable interactive primitives. Everything the chrome does is built from
// these three pieces plus <Icon>, so hover/pressed/disabled/focus states stay
// identical across regions.
import React from 'react';
import { Icon } from '../icons.jsx';

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
}) {
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

/** A square icon-only button; `title` is required for accessibility. */
export function IconButton({ icon, label, iconSize = 18, size = 'md', variant = 'ghost', ...rest }) {
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

/** A ribbon command button: large icon over a label, AutoCAD-style. */
export function RibbonButton({ icon, label, hint, disabled, active, onClick }) {
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
