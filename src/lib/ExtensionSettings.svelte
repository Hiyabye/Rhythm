<script lang="ts">
  import type { Extension } from './types';
  import { sourceExtensions } from './extensions';

  // Every feature beyond the core shell is a built-in extension. Turning one off
  // hides it and stops its background work; nothing it stored is deleted.
  let { extensions, busy, ontoggle, compact = false }: {
    extensions: Extension[]; busy: boolean; ontoggle: (id: string, enabled: boolean) => void; compact?: boolean;
  } = $props();
  let groups = $derived([
    { title: '정보원', members: extensions.filter((extension) => sourceExtensions.some((source) => source.id === extension.id)) },
    { title: '기능', members: extensions.filter((extension) => !sourceExtensions.some((source) => source.id === extension.id)) },
  ]);
</script>

{#if !compact}<p class="footnote">켠 확장 기능만 메뉴, 오늘 화면과 설정에 나타납니다. 꺼도 보관한 공지, 저장한 항목과 설정은 그대로 남고 다시 켜면 이어서 쓸 수 있습니다.</p>{/if}
{#each groups as group (group.title)}
  <h3 class="settings-group-title">{group.title}</h3>
  <div class="settings-group">
    {#each group.members as extension (extension.id)}
      <div class="settings-row"><span class="row-label">{extension.display_name}<span class="row-detail">{extension.description}{#if !extension.available} 지금은 사용할 수 없습니다.{/if}</span></span><button class="switch" class:on={extension.enabled} role="switch" aria-checked={extension.enabled} aria-label={`${extension.display_name} 사용 설정`} disabled={busy || !extension.available} onclick={() => ontoggle(extension.id, !extension.enabled)}><span></span></button></div>
    {/each}
  </div>
{/each}
