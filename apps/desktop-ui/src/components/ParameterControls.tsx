// The active-tool parameter controls of the status bar: one bounded stepper (or
// one named chooser, or one colour swatch) per numeric parameter and one toggle
// chip per flag parameter.
//
// Split out of `StatusBar.tsx` so the bar itself stays a stateless projection of
// `cad_status`: a stepper of a **typed** parameter (a real length, e.g. the text
// height) owns the one piece of local state in the strip — the text the user is
// typing — while the number it renders still comes from the host. Nothing here
// mirrors a value and nothing here calls IPC: a step, a pick or a commit is handed
// to the shell (`onStep`/`onChoose`), which owns the pending delta and the single
// parameter IPC.
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { Toggle } from './primitives.tsx';
import { ColorField } from './ColorChooser.tsx';
import type { ComposingKeyEvent } from './key-events.ts';
import { useI18n } from '../context.ts';
import { aciChoice, selectedAciIndex } from '../color/color-value.ts';
import { placeContextMenu } from '../commands/context-menu.ts';
import type { MenuPlacement } from '../commands/context-menu.ts';
import { choiceGridFocus, isChoiceGridKey } from '../tools/choice-grid.ts';
import {
  NO_TOOL_PARAMETER_DRAFT, TOOL_PARAMETER_INACTIVE_KEY, cancelToolParameterDraft, commitToolParameterDraft,
  stepToolParameterDraft, typeToolParameterDraft,
} from '../tools/tool-parameters.ts';
import type {
  ToolParameterChoice, ToolParameterDraft, ToolParameterFlagView, ToolParameterNumberView,
} from '../tools/tool-parameters.ts';

/**
 * The bounded numeric control for one active-tool parameter.
 *
 * It is a pure view over the host value plus the one local draft: the rendered
 * number is always the one the last `cad_status` carried (or exactly what the user
 * is typing, never a guess). The effective target — the status value plus the
 * bounded pending delta of steps the host has not echoed back yet — drives the
 * `-`/`+` targets and the disabled state, so rapid clicks compose and the buttons
 * stop at the parameter's own bounds (and the pending cap) while the host is not
 * admitting commands.
 *
 * A `typed` parameter (the text height is the only one: a world-unit length whose
 * useful values a fixed step cannot reach) additionally accepts a typed decimal.
 * Enter or a blur commits it through the parameter's own guard — a blur matters,
 * because the user's next move is usually a click on the drawing; Escape drops
 * the draft and shows the host value again; the arrow keys step from the draft. A
 * value the host would refuse is refused **here**, before any write, with the
 * catalog's message — never clamped and never sent.
 */
export function ParameterStepper({ view, onStep }: { readonly view: ToolParameterNumberView; readonly onStep: (key: string, target: number) => void }) {
  const { t } = useI18n();
  const name = t(view.labelKey);
  const [draft, setDraft] = useState<ToolParameterDraft>(NO_TOOL_PARAMETER_DRAFT);
  const refusal = draft.messageKey ? t(draft.messageKey, { name }) : null;
  // The one step target per direction, shared by the `-`/`+` buttons and the arrow
  // keys. While the user is typing, the draft is the base (the same base the typed
  // commit would write), so a click right after typing continues from what they
  // typed instead of silently stepping the stale host value; otherwise it is the
  // view's own raw target built from the effective, pending-composed value. `null`
  // is a value the host refuses: the control is disabled, never clamped.
  const typing = draft.text !== null;
  const stepTargets = {
    down: typing
      ? stepToolParameterDraft(view.key, draft, view.effective, -1)
      : (view.canDecrement ? view.decrement : null),
    up: typing
      ? stepToolParameterDraft(view.key, draft, view.effective, 1)
      : (view.canIncrement ? view.increment : null),
  };
  const stepDown = view.disabled ? null : stepTargets.down;
  const stepUp = view.disabled ? null : stepTargets.up;
  // A step is an absolute target, so it discards a half-typed draft rather than
  // leaving the field showing text that disagrees with the value just sent.
  const step = (target: number): void => {
    setDraft(cancelToolParameterDraft());
    onStep(view.key, target);
  };
  const commit = (): void => {
    const result = commitToolParameterDraft(view.key, draft);
    if (result.kind === 'idle') return;
    setDraft(result.draft);
    if (result.kind === 'write') onStep(view.key, result.number);
  };
  const onKeyDown = (event: ComposingKeyEvent<HTMLInputElement>): void => {
    // An IME owns its Enter (including the legacy keyCode 229): never commit a
    // half-composed value, and never read the IME's Escape as a revert.
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      // Revert to the last host value instead of letting the global Escape blur
      // the field (which would commit what the user just asked to discard) or
      // cancel the running tool.
      event.preventDefault();
      event.stopPropagation();
      setDraft(cancelToolParameterDraft());
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      commit();
      return;
    }
    if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      const target = event.key === 'ArrowUp' ? stepUp : stepDown;
      if (target !== null) step(target);
    }
  };
  return (
    <span
      className={['status-stepper', view.disabled ? 'is-disabled' : '', refusal ? 'is-invalid' : ''].filter(Boolean).join(' ')}
      role="group"
      aria-label={name}
      title={refusal ?? t(view.hintKey)}
    >
      {view.icon ? <Icon name={view.icon as IconName} size={12} /> : null}
      <span className="status-stepper__label" aria-hidden="true">{name}</span>
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.tool_parameter.decrease', { name })}
        title={t('status.tool_parameter.decrease', { name })}
        disabled={stepDown === null}
        onClick={stepDown === null ? undefined : () => step(stepDown)}
      >
        <Icon name="minus" size={11} />
      </button>
      {view.typed ? (
        <input
          type="text"
          inputMode="decimal"
          className="status-stepper__input mono"
          value={draft.text ?? view.text}
          aria-label={name}
          aria-invalid={refusal ? true : undefined}
          autoComplete="off"
          spellCheck={false}
          disabled={view.disabled}
          onChange={event => {
            const value = event.target.value;
            setDraft(current => typeToolParameterDraft(current, value));
          }}
          onKeyDown={onKeyDown}
          onBlur={commit}
        />
      ) : (
        <span className="status-stepper__value mono">{view.text}</span>
      )}
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.tool_parameter.increase', { name })}
        title={t('status.tool_parameter.increase', { name })}
        disabled={stepUp === null}
        onClick={stepUp === null ? undefined : () => step(stepUp)}
      >
        <Icon name="plus" size={11} />
      </button>
      {/* The refusal is visible in the chip's title and announced here, so a
          keyboard/screen-reader user hears why nothing was written. */}
      {refusal ? <span className="sr-only" role="alert">{refusal}</span> : null}
    </span>
  );
}

/**
 * The named chooser for a closed-set parameter: one segment per implemented code.
 *
 * HATCH's predefined pattern is the only row that carries one today, and there the
 * three codes **are** the three patterns the host transcribed: the name is the
 * whole choice, and a number is exactly the part a user cannot be expected to
 * remember. So that row stops being a stepper — every implemented name is on screen
 * at once (nothing hides behind a popup, which on this surface would also be a
 * second native surface over the CAD window), one click picks any of them, and the
 * chosen one carries the same accent an ON chip uses.
 *
 * What it keeps from the stepper it replaces: a pick is an **absolute target**
 * dispatched through the one parameter channel (`onChoose(key, value)`), so the wire
 * still carries the integer 1..3 and no string ever travels; the offered set is the
 * spec's own `choices`, never a superset; nothing is mirrored locally; admission
 * disables it; and `selected` follows the status value plus the bounded pending
 * delta, so the host's echo (or its refusal) is what settles it.
 *
 * Keyboard: each segment is a real `<button>` with `aria-pressed`, so Tab reaches
 * the row and Enter/Space picks — the same vocabulary as the ON/OFF chips beside
 * it. (Naming it a radiogroup without roving tabindex would promise arrow keys it
 * does not have.)
 */
export function ParameterChoices({ view, onChoose }: { readonly view: ToolParameterNumberView; readonly onChoose: (key: string, target: number) => void }) {
  const { t } = useI18n();
  const name = t(view.labelKey);
  // A scoped row outside its sibling code (the pattern name while the fill is solid
  // or user-defined) stays visible but inert. The reason is the row's own hint,
  // prefixed by the one generic sentence, so the copy can never drift from the
  // parameter it is about.
  const title = view.inactive ? `${t(TOOL_PARAMETER_INACTIVE_KEY)} \u00b7 ${t(view.hintKey)}` : t(view.hintKey);
  return (
    <span
      className={['status-choices', view.inactive ? 'is-inactive' : ''].filter(Boolean).join(' ')}
      role="group"
      aria-label={name}
      title={title}
    >
      {view.icon ? <Icon name={view.icon as IconName} size={12} /> : null}
      <span className="status-choices__label" aria-hidden="true">{name}</span>
      {/* One segment per implemented code, in the host's own order. A choice is
          absolute, so the segment the row already stands on writes nothing. */}
      {(view.choices ?? []).map(choice => (
        <button
          key={choice.value}
          type="button"
          className={['status-choices__choice', choice.selected ? 'is-selected' : ''].filter(Boolean).join(' ')}
          aria-pressed={choice.selected}
          disabled={choice.disabled}
          onClick={choice.selected ? undefined : () => onChoose(view.key, choice.value)}
        >
          {t(choice.labelKey)}
        </button>
      ))}
    </span>
  );
}

/**
 * The compact **grid** chooser for a closed set whose codes are a position: MTEXT's
 * nine attachment codes (AutoCAD's DXF group `71` anchor grid).
 *
 * Why not the segmented chooser beside it: the nine names ("Top left" … "Bottom
 * right") are the whole meaning, but nine readable segments would be roughly
 * 600 px of status strip — the row would push the drafting aids off a narrow
 * window, and the strip already scrolls for HATCH's ten rows. So the row keeps one
 * 20 px chip showing the label and the name in force, and opens the nine names as
 * the **3×3 grid the codes describe**: the arrangement itself is the diagram, so a
 * user picks a corner by where it sits, not by remembering that `7` means
 * bottom-left.
 *
 * What it keeps from the chooser it sits beside: a pick is an absolute target
 * dispatched through the one parameter channel (`onChoose(key, value)`), so the
 * wire still carries the integer `1..9` and no string ever travels; the offered
 * set is the view's own `choices` (the host's closed set plus its catalog name),
 * never a superset; nothing is mirrored locally; admission disables the chip; and
 * `selected` follows the status value plus the bounded pending delta.
 *
 * Placement: the panel is `position: fixed` and placed by the shortcut menu's own
 * `placeContextMenu`, anchored at the chip's top-left — the status bar sits on the
 * window's bottom edge, so the panel opens upward and is clamped inside the window.
 * It is the same in-DOM overlay vocabulary as the shortcut menu (no second window,
 * no portal); a press outside, a window blur/resize or any scroll closes it, and
 * the press is not prevented, so it still does what it would have done underneath.
 *
 * Keyboard: on open, focus moves to the code in force. Arrow keys move in **space**
 * (see `choiceGridFocus`: no wrapping across a row edge), Home/End jump to the first
 * and last code, Enter/Space pick the focused cell, and Escape closes and returns
 * focus to the chip — with `stopPropagation`, so the shell's Escape does not also
 * cancel the running tool.
 */
export function ParameterGridChoices({ view, onChoose }: { readonly view: ToolParameterNumberView; readonly onChoose: (key: string, target: number) => void }) {
  const { t } = useI18n();
  const name = t(view.labelKey);
  const choices = view.choices ?? [];
  const selectedIndex = choices.findIndex(choice => choice.selected);
  const current = selectedIndex >= 0 ? t(choices[selectedIndex].labelKey) : view.text;
  const title = view.inactive ? `${t(TOOL_PARAMETER_INACTIVE_KEY)} \u00b7 ${t(view.hintKey)}` : t(view.hintKey);
  const [open, setOpen] = useState(false);
  // `null` until the panel has been measured: the first paint is invisible, so it
  // never flashes at an unclamped position (the shortcut menu's own rule).
  const [placed, setPlaced] = useState<MenuPlacement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const cellRefs = useRef<Array<HTMLButtonElement | null>>([]);
  // One stable id per row, so the chip can point at the panel it opens.
  const menuId = `status-grid-${view.key}`;

  const close = useCallback((restoreFocus: boolean): void => {
    setOpen(false);
    setPlaced(null);
    if (restoreFocus) triggerRef.current?.focus({ preventScroll: true });
  }, []);

  // Measure once per open (and again if a locale/label change resizes it).
  useLayoutEffect(() => {
    if (!open) return;
    const menu = menuRef.current;
    const trigger = triggerRef.current;
    if (!menu || !trigger) return;
    const size = menu.getBoundingClientRect();
    const anchor = trigger.getBoundingClientRect();
    setPlaced(placeContextMenu({
      anchor: { x: anchor.left, y: anchor.top },
      size: { width: size.width, height: size.height },
      viewport: { width: window.innerWidth, height: window.innerHeight },
    }));
  }, [open, t]);

  // Focus follows the code in force, so the first arrow key starts from the cell
  // the user is actually on (and a host echo keeps it there).
  useEffect(() => {
    if (!open || !placed) return;
    cellRefs.current[Math.max(0, selectedIndex)]?.focus({ preventScroll: true });
  }, [open, placed, selectedIndex]);

  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null): boolean => Boolean(target && typeof target === 'object'
      && (menuRef.current?.contains(target as Node) === true || triggerRef.current?.contains(target as Node) === true));
    const onPointerDown = (event: PointerEvent): void => { if (!inside(event.target)) close(false); };
    // The panel does not scroll itself, so any scroll moves the chip out from
    // under it (or the strip under the pointer): close rather than drift.
    const onScroll = (): void => close(false);
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('blur', onScroll);
    window.addEventListener('resize', onScroll);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('blur', onScroll);
      window.removeEventListener('resize', onScroll);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open, close]);

  const onMenuKeyDown = (event: ComposingKeyEvent<HTMLDivElement>): void => {
    // An IME owns its keys (the commit Enter included).
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      // The panel's Escape is its own: the shell must not also cancel the tool.
      event.stopPropagation();
      close(true);
      return;
    }
    if (event.key === 'Tab') {
      // Not a grid move: the browser's own focus order leaves the panel, and the
      // panel closes behind it (no prevented default, so focus really moves on).
      close(false);
      return;
    }
    if (!isChoiceGridKey(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const focused = cellRefs.current.findIndex(cell => cell === document.activeElement);
    cellRefs.current[choiceGridFocus(focused, event.key, choices.length)]?.focus({ preventScroll: true });
  };

  const pick = (choice: ToolParameterChoice): void => {
    // A pick is absolute: the code already in force writes nothing, it only closes.
    close(true);
    if (!choice.selected) onChoose(view.key, choice.value);
  };

  return (
    <span className={['status-grid', view.inactive ? 'is-inactive' : '', view.disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}>
      <button
        ref={triggerRef}
        type="button"
        className="status-grid__trigger"
        aria-haspopup="true"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        title={title}
        disabled={view.disabled}
        onClick={() => setOpen(value => !value)}
      >
        {view.icon ? <Icon name={view.icon as IconName} size={12} /> : null}
        {/* The chip reads the parameter's name and then the name of the code in
            force ("Attachment  Top left"), so the row never shows a bare number. */}
        <span className="status-grid__label">{name}</span>
        <span className="status-grid__value">{current}</span>
        <span className="status-grid__caret" aria-hidden="true">
          <Icon name="chevronUp" size={10} />
        </span>
      </button>
      {open ? (
        <div
          ref={menuRef}
          id={menuId}
          className={['status-grid__menu', placed ? '' : 'is-unplaced'].filter(Boolean).join(' ')}
          role="group"
          aria-label={name}
          style={{ left: `${placed?.left ?? 0}px`, top: `${placed?.top ?? 0}px` }}
          onKeyDown={onMenuKeyDown}
        >
          {choices.map((choice, index) => (
            <button
              key={choice.value}
              ref={node => { cellRefs.current[index] = node; }}
              type="button"
              className={['status-grid__choice', choice.selected ? 'is-selected' : ''].filter(Boolean).join(' ')}
              aria-pressed={choice.selected}
              disabled={choice.disabled}
              onClick={() => pick(choice)}
            >
              {t(choice.labelKey)}
            </button>
          ))}
        </div>
      ) : null}
    </span>
  );
}

/**
 * The colour swatch for an ACI-indexed parameter: HATCH's two gradient stops.
 *
 * The wire carries a bare integer ACI index, so this row must not offer a
 * spelling the channel cannot take: no free-form field (`custom` is absent), no
 * `#RRGGBB` and no ByLayer/ByBlock entries (`symbols` is empty). What opens is the
 * app's one in-DOM colour chooser showing the standard palette `1..=255` — index
 * `0` is absent from it because it is the ByBlock placeholder, not a colour, and
 * the command refuses it by name.
 *
 * The pick is translated back to the **integer** with the shared
 * [`selectedAciIndex`] and dispatched through the same `onChoose(key, value)`
 * funnel a named chooser uses, so the wire is byte-for-byte what a step would send
 * and nothing string-shaped can leak into a numeric parameter. A pick that parses
 * to no index (or to the index already in force) sends nothing at all — the
 * chooser's own no-op rule.
 *
 * Layout: label + an 18px swatch trigger on the same 20px chip as the steppers, so
 * the strip keeps one control height at ten rows. It paints the colour the row
 * currently stands on (status value plus the bounded pending delta, like a
 * chooser's selected segment), never a fabricated one. A scoped row outside
 * `hatchPatternType === 3` is muted with the same "not in effect" sentence and
 * still opens: the palette is discoverable before the fill is switched, and the
 * host would accept the write either way.
 */
export function ParameterSwatch({ view, onChoose }: { readonly view: ToolParameterNumberView; readonly onChoose: (key: string, target: number) => void }) {
  const { t } = useI18n();
  const name = t(view.labelKey);
  const title = view.inactive ? `${t(TOOL_PARAMETER_INACTIVE_KEY)} \u00b7 ${t(view.hintKey)}` : t(view.hintKey);
  return (
    <div
      className={['status-swatch', view.inactive ? 'is-inactive' : '', view.disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      role="group"
      aria-label={name}
      title={title}
    >
      {view.icon ? <Icon name={view.icon as IconName} size={12} /> : null}
      <span className="status-swatch__label" aria-hidden="true">{name}</span>
      <ColorField
        // The chooser speaks the colour grammar (`aci:N`); the row's own value is
        // the integer, so the adapter lives here and nowhere else.
        value={aciChoice(view.effective)}
        label={t('status.tool_parameter.choose_color', { name })}
        symbols={[]}
        busy={view.disabled}
        onPick={next => {
          const index = selectedAciIndex(next);
          if (index === null || index === view.effective) return;
          onChoose(view.key, index);
        }}
      />
    </div>
  );
}

// The toggle chip for one active-tool flag parameter, using the same status-bar
// chip vocabulary as OSNAP / hover preview. It dispatches the *opposite* of the
// value the last `cad_status` carried — never a fabricated one.
//
// A flag scoped to a sibling code (HATCH `double`, which only the two pattern
// fills consult) keeps its chip and its dispatch and is only muted: the host
// accepts the write whatever the current fill is, so refusing it here would be a
// capability claim the host does not make. The title carries the one generic
// "not in effect" sentence plus the row's own hint, the same words the inert
// choosers and swatches use.
export function ParameterFlag({ view, onToggle }: { readonly view: ToolParameterFlagView; readonly onToggle: (key: string, next: boolean) => void }) {
  const { t } = useI18n();
  const title = view.inactive ? `${t(TOOL_PARAMETER_INACTIVE_KEY)} \u00b7 ${t(view.hintKey)}` : t(view.hintKey);
  return (
    <span className={['status-flag', view.inactive ? 'is-inactive' : ''].filter(Boolean).join(' ')}>
      <Toggle
        label={t(view.labelKey)}
        on={view.value}
        disabled={view.disabled}
        title={title}
        onClick={() => onToggle(view.key, view.next)}
      />
    </span>
  );
}
