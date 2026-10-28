// The command line's text field, shared by both presentations.
//
// The field carries more wiring than a plain input: the accessible label changes
// with the mode, the autocomplete/activedescendant attributes only exist while
// the list is open, and an over-long text step is `aria-invalid`. Every one of
// those comes from the controller, so the desktop region and the mobile bar
// cannot describe themselves differently. There is no `aria-describedby` any
// more: the sentence it pointed at was the refusal line under the field, and that
// line is gone — the history holds the words now, not the field.
//
// `className` is the only visual input: the docked field and the 44px mobile
// field are the same element with different CSS.
import React from 'react';
import { useI18n } from '../context.ts';
import { TEXT_STEP_PROMPT_KEY } from '../tools/text.ts';
import type { CommandLineController } from '../commands/use-command-line.ts';

export interface CommandFieldProps {
  /** The field's DOM id (both presentations use `cad-command`: one is mounted). */
  readonly id: string;
  /** The presentation's own class (sizing, surface, type scale). */
  readonly className: string;
  readonly line: CommandLineController;
  /** The virtual keyboard's action key; `send` on the mobile bar. */
  readonly enterKeyHint?: 'send';
}

export function CommandField({ id, className, line, enterKeyHint }: CommandFieldProps) {
  const { t } = useI18n();
  return (
    <>
      <label className="sr-only" htmlFor={id}>{line.textMode ? t(TEXT_STEP_PROMPT_KEY) : t('command.title')}</label>
      <input
        id={id}
        ref={line.inputRef}
        className={className}
        type="text"
        value={line.value}
        placeholder={line.placeholder}
        autoComplete="off"
        spellCheck={false}
        disabled={line.disabled}
        enterKeyHint={enterKeyHint}
        aria-autocomplete={line.autoComplete}
        aria-expanded={line.expanded}
        aria-activedescendant={line.activeOptionId}
        aria-invalid={line.invalid ? true : undefined}
        onChange={event => line.onChange(event.target.value)}
        onFocus={line.onFocus}
        onBlur={line.onBlur}
        onKeyDown={line.onKeyDown}
      />
    </>
  );
}
