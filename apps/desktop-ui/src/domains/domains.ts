// Application domains: the four interface profiles the chrome can render.
//
// The **document** owns the value (`document.json` `application_domain`, ADR-0053
// D2); this module only maps the stable token onto the presentation profile. The
// token set is the same one the Rust side validates, and an unknown value never
// becomes a guessed profile: a status that carries anything else reads as the
// drafting profile, which is also the document-less default.
//
// Pass 1 renders the drafting profile exactly as before and the other three as
// **explicit placeholders**: their chrome region (ribbon, command line) is not
// drawn and a card names the missing profile, instead of showing AutoCAD tools
// over a BIM document. No commands are added by a profile; enablement stays a
// document-capability fact (ADR-0053 D5).

import type { Invoke } from '../host/invoke.ts';

/** The stable domain tokens, in the order the switcher lists them. */
export type DomainId = 'cad2d' | 'bim' | 'concept' | 'mechanical';

/** Every domain this build models. */
export const DOMAIN_IDS: readonly DomainId[] = Object.freeze([
  'cad2d',
  'bim',
  'concept',
  'mechanical',
]);

/** The domain-less default: drafting, the only fully wired profile. */
export const DEFAULT_DOMAIN: DomainId = 'cad2d';

/** One presentation profile. */
export interface DomainProfile {
  readonly id: DomainId;
  /** The catalog key of the reader-facing name. */
  readonly labelKey: string;
  /** The catalog key of the reference application (tooltip/docs only). */
  readonly referenceKey: string;
  /** Whether this profile draws the ribbon band. */
  readonly ribbon: boolean;
  /** Whether the typing command line is part of the profile. */
  readonly commandLine: boolean;
  /**
   * Whether the profile is an explicit placeholder (no domain commands wired).
   * A placeholder hides its chrome and shows the domain card; it never claims
   * the reference application's tools.
   */
  readonly placeholder: boolean;
}

const profile = (
  id: DomainId,
  ribbon: boolean,
  commandLine: boolean,
  placeholder: boolean,
): DomainProfile => Object.freeze({
  id,
  labelKey: `domain.name.${id}`,
  referenceKey: `domain.reference.${id}`,
  ribbon,
  commandLine,
  placeholder,
});

/** The four profiles, frozen by token. */
export const DOMAIN_PROFILES: Readonly<Record<DomainId, DomainProfile>> = Object.freeze({
  cad2d: profile('cad2d', true, true, false),
  bim: profile('bim', false, false, true),
  concept: profile('concept', false, false, true),
  mechanical: profile('mechanical', false, false, true),
});

/** A `cad_status.applicationDomain` value, but only for a known token. */
export function normalizeDomain(value: unknown): DomainId | null {
  return typeof value === 'string' && (DOMAIN_IDS as readonly string[]).includes(value)
    ? value as DomainId
    : null;
}

/**
 * The domain a status renders.
 *
 * A known token wins; absent (no document, an old payload) and unknown both read
 * as drafting. That is a display default, not a host claim: the document's own
 * value still lives in `document.json`.
 */
export function domainOf(status: { readonly applicationDomain?: unknown } | null | undefined): DomainId {
  return normalizeDomain(status?.applicationDomain) ?? DEFAULT_DOMAIN;
}

/** The profile for a status. */
export function domainProfile(status: { readonly applicationDomain?: unknown } | null | undefined): DomainProfile {
  return DOMAIN_PROFILES[domainOf(status)];
}

/**
 * Parse one typed `DOMAIN <token>` argument.
 *
 * The canonical tokens are the file tokens; the reference names are accepted as
 * **input aliases** only (never displayed, never written), because muscle memory
 * should not require memorising our neutral spelling. `null` is an unknown
 * argument and the caller refuses it by name.
 */
export function domainFromArgument(text: unknown): DomainId | null {
  const token = String(text ?? '').trim().toLowerCase();
  switch (token) {
    case 'cad2d': case 'autocad': case 'dwg': case 'dxf': case 'dgn':
      return 'cad2d';
    case 'bim': case 'revit': case 'rvt': case 'ifc':
      return 'bim';
    case 'concept': case 'sketchup': case 'skp': case 'su':
      return 'concept';
    case 'mechanical': case 'solidworks': case 'sw': case 'step':
      return 'mechanical';
    default:
      return null;
  }
}

/** The command-line word that opens the switcher or writes one domain. */
export const DOMAIN_COMMAND = 'DOMAIN';

/** What a typed `DOMAIN …` line asks the chrome to do. */
export type DomainCommand =
  | { readonly kind: 'open' }
  | { readonly kind: 'set'; readonly domain: DomainId }
  | { readonly kind: 'unknown'; readonly token: string };

/**
 * Parse one typed `DOMAIN` line.
 *
 * Bare `DOMAIN` opens the switcher (ADR-0053 stage 2); `DOMAIN <token>` writes
 * the document's domain when one is open and sets the session preview otherwise.
 * The token accepts the same reference-name aliases as [`domainFromArgument`],
 * but an unknown token is returned **verbatim** so the refusal can name it —
 * never silently treated as drafting. Anything that merely starts with the word
 * (`DOMAINX`) is not this command.
 */
export function domainCommand(text: unknown): DomainCommand | null {
  const line = String(text ?? '').trim();
  const upper = line.toUpperCase();
  if (upper === DOMAIN_COMMAND) return { kind: 'open' };
  if (!upper.startsWith(`${DOMAIN_COMMAND} `)) return null;
  // The argument keeps its typed spelling for the refusal; `domainFromArgument`
  // does the case-insensitive match and only canonical tokens are written.
  const argument = line.slice(DOMAIN_COMMAND.length).trim();
  if (!argument) return { kind: 'open' };
  const domain = domainFromArgument(argument);
  return domain === null
    ? { kind: 'unknown', token: argument }
    : { kind: 'set', domain };
}

/** The wire command that writes the document's domain. */
export const SET_DOMAIN_COMMAND = 'cad_set_application_domain';

/** The fate of one domain write (`accepted` means queued/applied). */
export type DomainWriteResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'failed'; readonly code: string };

/**
 * Ask the host to write one document domain.
 *
 * The chrome never flips its own value: the next `cad_status.applicationDomain`
 * reports the accepted token, exactly like the theme setting. A refusal returns
 * the stable message (the host's `domain.unknown` / `domain.busy` / a carrier
 * `unsupported-runtime` string) for the caller to show.
 */
export async function writeDomain(invoke: Invoke, id: DomainId): Promise<DomainWriteResult> {
  try {
    const response = await invoke(SET_DOMAIN_COMMAND, { domain: id });
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    return accepted ? { kind: 'accepted' } : { kind: 'failed', code: 'domain.rejected' };
  } catch (error) {
    return { kind: 'failed', code: error instanceof Error ? error.message : String(error) };
  }
}
