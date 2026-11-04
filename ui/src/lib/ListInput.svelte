<script lang="ts">
  import { formatList, parseList } from './list';

  interface Props {
    value: string[];
    placeholder: string;
  }

  let { value = $bindable(), placeholder }: Props = $props();

  // The text is only seeded from value. Reformatting it on every keystroke
  // would eat a trailing comma as it is typed.
  let text = $state(formatList(value));
</script>

<input type="text" class="form-control" {placeholder} bind:value={text} oninput={() => (value = parseList(text))} />
