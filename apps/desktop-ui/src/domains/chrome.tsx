// Domain-gated chrome regions.
//
// The wrappers keep the profile check out of `App.tsx` (which is at the
// file-size gate) and out of `Ribbon`/`CommandLine` (whose hooks must run
// unconditionally). A profile that does not draw a region returns `null`, so the
// region takes no space and the host's CAD partition rect is unaffected.
//
// A `surface: 'menu'` profile (`concept`) renders the slot model's menubar in
// the same top row; `surface: 'domain-ribbon'` (`bim`) renders the registered
// ribbon tabs through the same band geometry. That keeps the app's region order
// unchanged while the band's renderer is chosen by the profile.
import React from 'react';
import { useShell } from '../context.ts';
import { DOMAIN_PROFILES } from './domains.ts';
import { renderedDomain } from './store.ts';
import { CommandLine } from '../components/CommandLine.tsx';
import { Ribbon } from '../components/Ribbon.tsx';
import { MobileCommandSurface } from '../components/MobileCommandSurface.tsx';
import { DomainMenuBar } from './ui/DomainMenuBar.tsx';
import { DomainRibbonBand } from './ui/DomainRibbonBand.tsx';
import { useDomainUi } from './ui/use-ui-layout.ts';
// Side-effects: each wired profile registers its own slot data.
import './bim/register.ts';
import './concept/register.ts';
import './mechanical/register.ts';

/** The ribbon band or menubar, only when the current profile draws one. */
export function DomainRibbon() {
  const shell = useShell();
  // `renderedDomain` includes the document-less preview and sends an unwired
  // profile back to drafting, so the whole top region matches the rendered
  // interface (the fallback notice names the requested domain).
  const domain = renderedDomain(shell.status);
  const profile = DOMAIN_PROFILES[domain];
  const layout = useDomainUi(domain);
  if (profile.surface === 'menu') return <DomainMenuBar />;
  if (profile.surface === 'domain-ribbon') {
    return layout.ribbon && layout.ribbon.length > 0 ? <DomainRibbonBand tabs={layout.ribbon} /> : null;
  }
  if (!profile.ribbon) return null;
  return <Ribbon />;
}

/** The typing command line, only when the rendered profile includes one. */
export function DomainCommandLine() {
  const shell = useShell();
  if (!DOMAIN_PROFILES[renderedDomain(shell.status)].commandLine) return null;
  return <CommandLine />;
}

/** The compact command surface, only when the rendered profile includes one. */
export function DomainMobileCommandSurface() {
  const shell = useShell();
  if (!DOMAIN_PROFILES[renderedDomain(shell.status)].commandLine) return null;
  return <MobileCommandSurface />;
}
