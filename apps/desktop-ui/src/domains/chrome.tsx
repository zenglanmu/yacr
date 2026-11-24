// Domain-gated chrome regions.
//
// The wrappers keep the profile check out of `App.tsx` (which is at the
// file-size gate) and out of `Ribbon`/`CommandLine` (whose hooks must run
// unconditionally). A profile that does not draw a region returns `null`, so the
// region takes no space and the host's CAD partition rect is unaffected.
import React from 'react';
import { useShell } from '../context.ts';
import { domainProfile } from './domains.ts';
import { CommandLine } from '../components/CommandLine.tsx';
import { Ribbon } from '../components/Ribbon.tsx';
import { MobileCommandSurface } from '../components/MobileCommandSurface.tsx';

/** The ribbon band, only when the current profile draws one. */
export function DomainRibbon() {
  const shell = useShell();
  if (!domainProfile(shell.status).ribbon) return null;
  return <Ribbon />;
}

/** The typing command line, only when the current profile includes one. */
export function DomainCommandLine() {
  const shell = useShell();
  if (!domainProfile(shell.status).commandLine) return null;
  return <CommandLine />;
}

/** The compact command surface, only when the current profile includes one. */
export function DomainMobileCommandSurface() {
  const shell = useShell();
  if (!domainProfile(shell.status).commandLine) return null;
  return <MobileCommandSurface />;
}
