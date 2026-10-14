// @feature runtime
// @feature installation
// @spec docs/features/runtime.md
// @spec docs/features/installation.md

export interface NexusLaunchContext {
  executable: string;
  config?: string;
}

// `nexus setup` replaces this module in the installed Pi extension with the
// absolute executable and optional explicit configuration selected at setup.
export const nexusLaunchContext: NexusLaunchContext = {
  executable: "nexus",
};
