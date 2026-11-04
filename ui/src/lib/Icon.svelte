<script lang="ts" module>
  // Stroke icons on a 24px grid, drawn in currentColor.
  const paths = {
    search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    x: '<path d="M6 6l12 12M18 6 6 18"/>',
    check: '<path d="m5 12.5 4.5 4.5L19 7"/>',
    trash: '<path d="M4 7h16M9 7V4.5h6V7M6.5 7l1 13h9l1-13M10 11v5M14 11v5"/>',
    'chevron-right': '<path d="m9 6 6 6-6 6"/>',
    'chevron-down': '<path d="m6 9 6 6 6-6"/>',
    'arrow-left': '<path d="M19 12H5M11 6l-6 6 6 6"/>',
    'corner-down-left': '<path d="M20 5v7a3 3 0 0 1-3 3H5M9 11l-4 4 4 4"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M4.6 4.6 6 6M18 18l1.4 1.4M2.5 12h2M19.5 12h2M4.6 19.4 6 18M18 6l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8.5 8.5 0 0 1 9.5 4 7 7 0 1 0 20 14.5Z"/>',
    monitor: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
    'log-out': '<path d="M9 4H5.5A1.5 1.5 0 0 0 4 5.5v13A1.5 1.5 0 0 0 5.5 20H9M15 8l4 4-4 4M19 12H9"/>',
    users:
      '<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0M16 4.6a3.5 3.5 0 0 1 0 6.8M18 14.2a6.5 6.5 0 0 1 3.5 5.8"/>',
    user: '<circle cx="12" cy="8" r="4"/><path d="M4.5 20.5a7.5 7.5 0 0 1 15 0"/>',
    repo: '<path d="M5 18.5V5.5A2.5 2.5 0 0 1 7.5 3H19v14H7.5A2.5 2.5 0 0 0 5 19.5 2.5 2.5 0 0 0 7.5 22H19v-5"/>',
    tag: '<path d="M3 12V4.5A1.5 1.5 0 0 1 4.5 3H12l9 9-9 9Z"/><circle cx="8" cy="8" r="1.5"/>',
    hash: '<path d="M5 9h15M4 15h15M10 3 8 21M16 3l-2 18"/>',
    external: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
    merge: '<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 7v10M7.5 6.5C11 9 13 12 16 12"/>',
    branch: '<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="7" r="2"/><path d="M6 7v10M18 9c0 5-6 4-11 8.5"/>',
    bell: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15ZM10 21h4"/>',
    'bell-off': '<path d="M8.5 5.5A6 6 0 0 1 18 11v4M6 11v5l-1.5 2H17M10 21h4M3 3l18 18"/>',
    alert: '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>',
    message: '<path d="M4 5h16v11H9l-5 4Z"/>',
    command: '<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3Z"/>',
    commit: '<circle cx="12" cy="12" r="3.5"/><path d="M3 12h5.5M15.5 12H21"/>',
    loader: '<path d="M21 12a9 9 0 1 1-9-9"/>',
  };

  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    size?: number;
    spin?: boolean;
  }

  let { name, size = 16, spin = false }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class:spin
>
  {@html paths[name]}
</svg>

<style>
  svg {
    flex: none;
  }

  .spin {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
