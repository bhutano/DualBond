<script lang="ts">
  type IconName =
    | "activity"
    | "refresh"
    | "settings"
    | "download"
    | "pause"
    | "play"
    | "close"
    | "folder"
    | "plus"
    | "wifi"
    | "check"
    | "trash"
    | "globe";

  let { name, size = 18 }: { name: IconName; size?: number } = $props();
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
>
  {#if name === "activity"}
    <path d="M3 12h4l2.2-6 4.2 12 2.2-6H21" />
  {:else if name === "refresh"}
    <path d="M20 6v5h-5" /><path d="M4 18v-5h5" />
    <path d="M6.1 9a7 7 0 0 1 11.5-2.4L20 11M4 13l2.4 4.4A7 7 0 0 0 17.9 15" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z" />
  {:else if name === "download"}
    <path d="M12 3v12" /><path d="m7 10 5 5 5-5" /><path d="M5 21h14" />
  {:else if name === "pause"}
    <path d="M8 5v14M16 5v14" />
  {:else if name === "play"}
    <path d="m8 5 11 7-11 7Z" />
  {:else if name === "close"}
    <path d="m6 6 12 12M18 6 6 18" />
  {:else if name === "folder"}
    <path d="M3 7h7l2 2h9v10H3Z" /><path d="M3 7V5h7l2 2" />
  {:else if name === "plus"}
    <path d="M12 5v14M5 12h14" />
  {:else if name === "wifi"}
    <path d="M4.5 9.5a11 11 0 0 1 15 0M7.5 13a6.7 6.7 0 0 1 9 0M10.4 16.4a2.4 2.4 0 0 1 3.2 0" /><circle cx="12" cy="19" r=".7" fill="currentColor" stroke="none" />
  {:else if name === "check"}
    <path d="m5 12 4 4L19 6" />
  {:else if name === "trash"}
    <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
  {:else if name === "globe"}
    <circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c2.6 2.7 3.9 5.7 3.9 9s-1.3 6.3-3.9 9c-2.6-2.7-3.9-5.7-3.9-9S9.4 5.7 12 3Z" />
  {/if}
</svg>
