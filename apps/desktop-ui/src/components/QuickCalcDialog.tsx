// QuickCalc: the four-function calculator dialog.
//
// It offers exactly what the engine implements — `+ - * /`, parentheses, a
// decimal point, a sign flip, clear, backspace, memory (M+/MR/MC) and copying
// the result. There are no engineering, trigonometric, matrix or unit
// functions; a button for one would be a promise this build cannot keep, so the
// grid simply does not carry them.
//
// The expression field is editable: typing works exactly like pressing the keys
// (a sanitized infix expression), `Enter` evaluates, and Escape closes through
// the shared dialog focus hook (focus trap, restore on close).
import React, { useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { writeClipboardText } from '../clipboard.ts';
import {
  INITIAL_CALC_STATE,
  appendCalcToken,
  backspaceCalc,
  calcCopyText,
  calcDisplay,
  calcMemoryActive,
  calcPreview,
  clearCalc,
  evaluateCalc,
  formatCalcNumber,
  memoryAddCalc,
  memoryClearCalc,
  memoryRecallCalc,
  setCalcExpression,
  toggleSignCalc,
} from '../chrome/quick-calc.ts';
import type { CalcState } from '../chrome/quick-calc.ts';
import {
  DIALOG_BODY,
  DIALOG_CARD,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';
import { useConfirmDialogFocus } from './confirm-focus.ts';

/** One keypad key: a label plus the action it runs. */
interface CalcKey {
  readonly key: string;
  readonly labelKey: string | null;
  readonly action: (state: CalcState) => CalcState;
  /** Insert this token instead of running a transition. */
  readonly token?: string;
}

const KEYS: readonly (readonly CalcKey[])[] = Object.freeze([
  [
    { key: 'MC', labelKey: 'quickcalc.memory_clear', action: memoryClearCalc },
    { key: 'MR', labelKey: 'quickcalc.memory_recall', action: memoryRecallCalc },
    { key: 'M+', labelKey: 'quickcalc.memory_add', action: memoryAddCalc },
    { key: 'C', labelKey: 'quickcalc.clear', action: clearCalc },
  ],
  [
    { key: '(', labelKey: 'quickcalc.open_paren', action: state => state, token: '(' },
    { key: ')', labelKey: 'quickcalc.close_paren', action: state => state, token: ')' },
    { key: '⌫', labelKey: 'quickcalc.backspace', action: backspaceCalc },
    { key: '/', labelKey: 'quickcalc.divide', action: state => state, token: '/' },
  ],
  [
    { key: '7', labelKey: null, action: state => state, token: '7' },
    { key: '8', labelKey: null, action: state => state, token: '8' },
    { key: '9', labelKey: null, action: state => state, token: '9' },
    { key: '×', labelKey: 'quickcalc.multiply', action: state => state, token: '*' },
  ],
  [
    { key: '4', labelKey: null, action: state => state, token: '4' },
    { key: '5', labelKey: null, action: state => state, token: '5' },
    { key: '6', labelKey: null, action: state => state, token: '6' },
    { key: '-', labelKey: 'quickcalc.minus', action: state => state, token: '-' },
  ],
  [
    { key: '1', labelKey: null, action: state => state, token: '1' },
    { key: '2', labelKey: null, action: state => state, token: '2' },
    { key: '3', labelKey: null, action: state => state, token: '3' },
    { key: '+', labelKey: 'quickcalc.plus', action: state => state, token: '+' },
  ],
  [
    { key: '±', labelKey: 'quickcalc.plus_minus', action: toggleSignCalc },
    { key: '0', labelKey: null, action: state => state, token: '0' },
    { key: '.', labelKey: 'quickcalc.decimal', action: state => state, token: '.' },
    { key: '=', labelKey: 'quickcalc.equals', action: evaluateCalc },
  ],
]);

type CopyState = 'idle' | 'copied' | 'failed';

/** The dialog's fixed card size (logical px, clamped to the viewport). */
const CARD: React.CSSProperties = Object.freeze({
  ...DIALOG_CARD,
  display: 'flex',
  flexDirection: 'column',
  width: 'min(384px, calc(100vw - 32px))',
  maxHeight: 'calc(100vh - 32px)',
  boxShadow: '0 18px 48px rgba(0, 0, 0, .45)',
});

interface QuickCalcDialogProps {
  readonly onClose: () => void;
}

export function QuickCalcDialog({ onClose }: QuickCalcDialogProps) {
  const { t } = useI18n();
  const [calc, setCalc] = useState<CalcState>(INITIAL_CALC_STATE);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  useConfirmDialogFocus({ dialog: dialogRef, initial: inputRef }, onClose);

  const preview = calcPreview(calc);
  const display = calcDisplay(calc);

  function run(key: CalcKey): void {
    setCopyState('idle');
    setCalc(state => {
      if (key.token !== undefined) return appendCalcToken(state, key.token);
      if (key.key === '=') return evaluateCalc(state);
      return key.action(state);
    });
  }

  async function copyResult(): Promise<void> {
    try {
      const copied = await writeClipboardText(calcCopyText(calc));
      setCopyState(copied ? 'copied' : 'failed');
    } catch {
      setCopyState('failed');
    }
  }

  const body = (
    <div className="popup-overlay chrome-dialog-overlay" data-yacr-dialog="quickcalc">
      <div
        ref={dialogRef}
        className="popup-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t('quickcalc.title')}
        style={CARD}
      >
        <header className="palette__head" style={DIALOG_HEAD}>
          <h2 style={DIALOG_TITLE}>
            <Icon name="settings" size={15} />
            {t('quickcalc.title')}
          </h2>
          <IconButton icon="close" size="xs" label={t('quickcalc.close')} onClick={onClose} />
        </header>
        <div style={DIALOG_BODY}>
          <div className="quickcalc">
            <input
              ref={inputRef}
              className="quickcalc__display"
              value={display}
              placeholder="0"
              aria-label={t('quickcalc.expression')}
              inputMode="decimal"
              autoComplete="off"
              spellCheck={false}
              onChange={event => {
                setCalc(state => setCalcExpression(state, event.target.value));
                setCopyState('idle');
              }}
              onKeyDown={event => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  setCalc(state => evaluateCalc(state));
                  setCopyState('idle');
                }
              }}
            />
            <div className="quickcalc__meta">
              {calc.error ? (
                <span className="quickcalc__error" role="alert">{t('quickcalc.error')}</span>
              ) : preview !== null ? (
                <span className="quickcalc__preview" aria-live="polite">= {preview}</span>
              ) : (
                <span className="quickcalc__preview is-empty" aria-hidden="true">&nbsp;</span>
              )}
              <span className={['quickcalc__memory', calcMemoryActive(calc) ? 'is-active' : ''].filter(Boolean).join(' ')}>
                {calcMemoryActive(calc) ? `M ${formatCalcNumber(calc.memory)}` : t('quickcalc.memory_empty')}
              </span>
            </div>
            <div className="quickcalc__keys">
              {KEYS.flat().map(key => (
                <button
                  key={key.key}
                  type="button"
                  className={[
                    'quickcalc__key',
                    key.labelKey === null ? '' : 'is-action',
                    key.key === '=' ? 'is-equals' : '',
                  ].filter(Boolean).join(' ')}
                  aria-label={key.labelKey === null ? key.key : t(key.labelKey)}
                  onClick={() => run(key)}
                >
                  {key.labelKey === null ? key.key : t(key.labelKey)}
                </button>
              ))}
            </div>
          </div>
        </div>
        <footer style={DIALOG_FOOT}>
          {copyState === 'copied' ? <span className="quickcalc__copied" role="status">{t('quickcalc.copied')}</span> : null}
          {copyState === 'failed' ? <span className="quickcalc__copy-failed" role="alert">{t('quickcalc.copy_failed')}</span> : null}
          <Button variant="chip" icon="copy" iconSize={14} onClick={() => { void copyResult(); }}>
            {t('quickcalc.copy')}
          </Button>
          <Button variant="solid" onClick={onClose}>{t('quickcalc.close')}</Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? body : createPortal(body, document.body);
}
