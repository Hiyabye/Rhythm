<script lang="ts">
  export let name = 'today';
  export let size = 20;
  const paths: Record<string, string> = {
    today: 'M12 3v2m0 14v2M3 12h2m14 0h2M5.6 5.6 7 7m10 10 1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z',
    inbox: 'M4 4h16v16H4V4Zm0 10h5l1 3h4l1-3h5',
    search: 'm21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
    bookmark: 'M6 3h12v18l-6-4-6 4V3Z',
    settings: 'M12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8Zm-3-5h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1 1-3Z',
    arrow: 'M4 12h16m-6-6 6 6-6 6',
    check: 'm5 12 4 4L19 6',
    lock: 'M6 10h12v11H6V10Zm3 0V6a3 3 0 0 1 6 0v4m-3 4v3',
    leaf: 'M20 3C8 3 3 7 5 14c3 7 15 5 15-11ZM4 21 15 10',
    moon: 'M20 14a8 8 0 0 1-10-10 9 9 0 1 0 10 10Z',
    monitor: 'M3 3h18v13H3V3Zm5 18h8m-4-5v5',
    cpu: 'M6 6h12v12H6V6Zm3 3h6v6H9V9ZM9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4m12-6h4m-4 6h4',
    info: 'M12 11v6m0-10v.1M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z',
    refresh: 'M20 7v5h-5M4 17v-5h5M6 6a8 8 0 0 1 14 6M4 12a8 8 0 0 0 14 6',
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name] ?? paths.info} />
</svg>
