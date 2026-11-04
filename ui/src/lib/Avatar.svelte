<script lang="ts">
  interface Props {
    name: string;
    size?: number;
  }

  let { name, size = 32 }: Props = $props();

  // A stable hue per name, so a person keeps their color across pages.
  let hue = $derived([...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
  let initials = $derived(
    name
      .split(/[-_.\s]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((s) => s[0].toUpperCase())
      .join('') || '?',
  );
</script>

<span
  class="avatar"
  style:--hue={hue}
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.38)}px"
  aria-hidden="true">{initials}</span
>

<style>
  .avatar {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: hsl(var(--hue) 70% 92%);
    color: hsl(var(--hue) 45% 35%);
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  :global([data-theme='dark']) .avatar {
    background: hsl(var(--hue) 35% 22%);
    color: hsl(var(--hue) 70% 78%);
  }
</style>
