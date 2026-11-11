// React bindings for the two pure chrome stores.
//
// The stores themselves (`workspaces.ts`, `surfaces.ts`) are framework-free so
// the Node contract tests can drive them directly. These two hooks are the only
// React entry points, and both use `useSyncExternalStore` with the snapshot
// function as the server snapshot too: the chrome renders the same value on every
// carrier, and no store is ever mutated during render.
import { useSyncExternalStore } from 'react';
import { subscribeWorkspaces, workspaceSnapshot } from './workspaces.ts';
import type { WorkspaceSnapshot } from './workspaces.ts';
import { chromeSurfaceSnapshot, subscribeChromeSurfaces } from './surfaces.ts';
import type { ChromeSurfaceState } from './surfaces.ts';

/** The workspace presets/current/apply revision. */
export function useWorkspaceSnapshot(): WorkspaceSnapshot {
  return useSyncExternalStore(subscribeWorkspaces, workspaceSnapshot, workspaceSnapshot);
}

/** The open chrome-local surfaces. */
export function useChromeSurfaceState(): ChromeSurfaceState {
  return useSyncExternalStore(subscribeChromeSurfaces, chromeSurfaceSnapshot, chromeSurfaceSnapshot);
}
