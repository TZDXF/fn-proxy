<script setup lang="ts">
defineProps<{ name: string; size?: number }>();
const paths: Record<string, string> = {
  "window-minimize": "M5 12h14",
  "window-maximize": "M5 5h14v14H5z",
  "window-restore": "M9 9h10v10H9z M5 15V5h10",
  settings:
    "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M12 2v2 M12 20v2 M2 12h2 M20 12h2 M5 5l1.5 1.5 M17.5 17.5 19 19 M5 19l1.5-1.5 M17.5 6.5 19 5",
  moon: "M20.5 13a9 9 0 1 1-9.5-9.5A7 7 0 0 0 20.5 13z",
  monitor: "M3 3h18v14H3z M8 21h8 M12 17v4",
  grid: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  server: "M4 3h16v7H4z M4 14h16v7H4z M7 6.5h.01 M7 17.5h.01 M11 6.5h6 M11 17.5h6",
  link: "M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-2 2 M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l2-2",
  shield: "M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6z M8 12l3 3 5-6",
  code: "m8 5-6 7 6 7 M16 5l6 7-6 7 M14 3l-4 18",
  terminal: "m4 5 6 7-6 7 M13 19h7",
  arrow: "M4 12h16 m-6-6 6 6-6 6",
  plus: "M12 4v16 M4 12h16",
  refresh: "M20 7v5h-5 M4 17v-5h5 M5 7a8 8 0 0 1 13-2l2 2 M19 17a8 8 0 0 1-13 2l-2-2",
  copy: "M9 9h12v12H9z M15 9V3H3v12h6",
  external: "M14 3h7v7 M21 3l-9 9 M10 3H3v18h18v-7",
  lock: "M5 11h14v10H5z M8 11V7a4 4 0 0 1 8 0v4 M12 15v2",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
  check: "m5 12 4 4L19 6",
  close: "m6 6 12 12 M6 18 18 6",
  trash: "M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M10 10v7 M14 10v7",
  edit: "m15 4 5 5 M4 15 16 3l5 5L9 20H4z",
  power: "M12 2v10 M6 5a9 9 0 1 0 12 0",
  info: "M12 8h.01 M12 11v6 M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
  globe: "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0 M2 12h20 M12 2c5 6 5 14 0 20-5-6-5-14 0-20",
  diamond: "M3 8l4-5h10l4 5-9 13z M3 8h18 M7 3l5 18 5-18",
  speed: "M4 18a9 9 0 1 1 16 0 M12 14l5-6 M5 10l2 1 M12 4v3 M18 10l2-1 M10 18h4",
  transfer: "M8 3v17 m-4-4 4 4 4-4 M16 21V4 m-4 4 4-4 4 4",
  calendar: "M4 5h16v16H4z M4 10h16 M8 3v4 M16 3v4 M8 14h2 M14 14h2",
  folder: "M3 7V4h7l2 3h9v14H3z",
};
</script>
<template>
  <svg
    :width="size || 18"
    :height="size || 18"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="paths[name] || paths.info" />
  </svg>
</template>
