<script lang="ts" module>
  // Original line icons on a 20x20 grid, drawn for this app.
  const PATHS: Record<string, string> = {
    sortAsc: "M5 3v14 M2 14l3 3 3-3 M10 5h3 M10 9h5 M10 13h7",
    sortDesc: "M5 17v-14 M2 6l3-3 3 3 M10 5h7 M10 9h5 M10 13h3",
    filter: "M3 4h14l-5.5 6.5v5.5l-3 1.5v-7z",
    find: "M8.5 3.5a5 5 0 1 1-.1 0z M12 12l5 5",
    new: "M5 2.5h7l3.5 3.5v11.5h-10.5z M12 2.5v3.5h3.5 M10 9v6 M7 12h6",
    open: "M2.5 5.5v10.5h13l2-7h-13l-2 7 M2.5 5.5h5l1.5 1.5h6v2",
    save: "M3.5 3.5h10l3 3v10h-13z M6.5 3.5v4h6v-4 M6.5 16.5v-5h7v5",
    undo: "M7 5l-3.5 3.5 3.5 3.5 M3.5 8.5h8a4.5 4.5 0 0 1 0 9h-3",
    redo: "M13 5l3.5 3.5-3.5 3.5 M16.5 8.5h-8a4.5 4.5 0 0 0 0 9h3",
    cut: "M6 15.5a2 2 0 1 1-.1 0z M14 15.5a2 2 0 1 1-.1 0z M7.5 14l7-10.5 M12.5 14l-7-10.5",
    copy: "M7 7h9.5v9.5h-9.5z M13 7v-3.5h-9.5v9.5h3.5",
    paste: "M6 4.5h-2.5v13h13v-13h-2.5 M7 3h6v3h-6z",
    rowAbove: "M3 10.5h14v6h-14z M10 2.5v6 M7 5.5h6",
    rowBelow: "M3 3.5h14v6h-14z M10 11.5v6 M7 14.5h6",
    rowDelete: "M3 7h14v6h-14z M7.5 8.5l5 3 M12.5 8.5l-5 3",
    colLeft: "M10.5 3h6v14h-6z M2.5 10h6 M5.5 7v6",
    colRight: "M3.5 3h6v14h-6z M11.5 10h6 M14.5 7v6",
    colDelete: "M7 3h6v14h-6z M8.5 7.5l3 5 M11.5 7.5l-3 5",
    freeze: "M3 3h14v14h-14z M3 8h14 M8 3v14",
    header: "M3 3h14v14h-14z M3 7.5h14 M6 5.3h8",
    zoomIn: "M8.5 3a5.5 5.5 0 1 1-.1 0z M12.5 12.5l4.5 4.5 M6 8.5h5 M8.5 6v5",
    zoomOut: "M8.5 3a5.5 5.5 0 1 1-.1 0z M12.5 12.5l4.5 4.5 M6 8.5h5",
    panel: "M3 3h14v14h-14z M3 12.5h14 M5.5 14.8h6",
    theme: "M10 3a7 7 0 1 0 0 14z M10 3a7 7 0 0 1 0 14",
    close: "M5.5 5.5l9 9 M14.5 5.5l-9 9",
  };
</script>

<script lang="ts">
  interface Props {
    name: string;
    size?: number;
  }
  let { name, size = 18 }: Props = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
  <path d={PATHS[name] ?? ""} />
</svg>
