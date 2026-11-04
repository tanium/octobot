<script lang="ts">
  interface Props {
    text: string;
    // Indexes of matched characters, from fuzzy().
    positions: number[];
  }

  let { text, positions }: Props = $props();

  // Runs of matched and unmatched text, so each run is one element.
  let runs = $derived.by(() => {
    const hit = new Set(positions);
    const out: { text: string; hit: boolean }[] = [];
    for (let i = 0; i < text.length; i++) {
      const h = hit.has(i);
      const last = out[out.length - 1];
      if (last && last.hit === h) {
        last.text += text[i];
      } else {
        out.push({ text: text[i], hit: h });
      }
    }
    return out;
  });
</script>

{#each runs as run, i (i)}{#if run.hit}<mark>{run.text}</mark>{:else}{run.text}{/if}{/each}
