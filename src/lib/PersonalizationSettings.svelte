<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { PersonalInterest, PersonalInterestInput, PersonalizationSnapshot } from './types';
  import { archiveError } from './archive-format';
  import { confirmAction } from './confirm';
  import Hint from './Hint.svelte';
  import Popover from './Popover.svelte';

  type Polarity = PersonalInterest['polarity'];
  let { desktop }: { desktop: boolean } = $props();
  let data = $state<PersonalizationSnapshot | null>(null);
  let busy = $state(false);
  let resetPending = $state(false);
  let error = $state('');
  let notice = $state('');
  let university = $state('');
  let department = $state('');
  let studyLevel = $state('');
  // Quick-add fields, one per direction.
  let drafts = $state<Record<Polarity, string>>({ positive: '', negative: '' });
  // The chip editor, shown in the popover of the interest being edited.
  let editingId = $state<number | null>(null);
  let name = $state('');
  let aliases = $state('');
  let priority = $state(3);
  let polarity = $state<Polarity>('positive');
  let interestDraftRevision = $state(0);
  let parsedAliases = $derived(aliases.split(/[,，;；\n]/).map((item) => item.trim()).filter(Boolean));
  let invalidAliases = $derived(parsedAliases.length > 12 || parsedAliases.some((alias) => Array.from(alias).length > 80));
  let explicit = $derived(data?.interests.filter((interest) => interest.origin === 'explicit') ?? []);
  let learned = $derived(data?.interests.filter((interest) => interest.origin === 'learned') ?? []);

  onMount(() => {
    if (desktop) void refresh();
  });

  async function refresh() {
    try {
      data = await invoke<PersonalizationSnapshot>('personalization_get');
      setProfile(data.profile);
    } catch (reason) {
      error = archiveError(reason);
    }
  }

  function setProfile(value: PersonalizationSnapshot['profile']) {
    university = value.university;
    department = value.department;
    studyLevel = value.study_level;
  }

  async function action(command: string, args: Record<string, unknown> = {}): Promise<boolean> {
    if (busy) return false;
    busy = true;
    error = '';
    notice = '';
    try {
      data = await invoke<PersonalizationSnapshot>(command, args);
      notice = '개인화 설정을 저장했습니다.';
      return true;
    } catch (reason) {
      error = archiveError(reason);
      return false;
    } finally {
      busy = false;
    }
  }

  async function saveProfile() {
    const profile = { university, department, study_level: studyLevel };
    if (await action('personalization_save_profile', { profile })
      && university === profile.university
      && department === profile.department
      && studyLevel === profile.study_level
      && data) {
      setProfile(data.profile);
    }
  }

  async function addInterest(direction: Polarity) {
    const value = drafts[direction].trim();
    if (!value) return;
    const saved = await action('personalization_save_interest', {
      interest: { id: null, name: value, aliases: [], priority: 3, polarity: direction } satisfies PersonalInterestInput,
    });
    // Keep whatever was typed while the save was in flight.
    if (saved && drafts[direction].trim() === value) drafts[direction] = '';
  }

  async function saveInterest(interest: PersonalInterest) {
    const value = name.trim();
    if (!value || invalidAliases) return;
    const draftRevision = interestDraftRevision;
    const saved = await action('personalization_save_interest', {
      interest: { id: interest.id, name: value, aliases: parsedAliases, priority, polarity } satisfies PersonalInterestInput,
    });
    if (saved && interestDraftRevision === draftRevision) closeEditor(interest.id);
  }

  async function removeInterest(interest: PersonalInterest) {
    let confirmed: boolean;
    try {
      confirmed = await confirmAction(interest.origin === 'learned'
        ? `학습된 관심사 '${interest.name}'을(를) 삭제할까요?\n\n지금까지의 저장·관심 표시 기록만으로는 다시 만들어지지 않습니다. 보관한 공지와 피드백은 그대로 유지됩니다.`
        : `관심사 '${interest.name}'을(를) 삭제할까요? 추천에서 이 관심사를 더 이상 반영하지 않습니다. 보관한 공지는 그대로 유지됩니다.`, '삭제');
    } catch (reason) {
      error = `${archiveError(reason)} 확인 창을 열지 못해 삭제하지 않았습니다.`;
      return;
    }
    if (!confirmed) return;
    if (await action('personalization_remove_interest', { id: interest.id }) && editingId === interest.id) editingId = null;
  }

  async function resetPersonalization() {
    let confirmed: boolean;
    try {
      confirmed = await confirmAction('개인화 프로필, 관심사, 학습된 기억과 공지별 피드백을 모두 지울까요? 지운 내용은 되돌릴 수 없습니다.\n\n보관한 공지, 읽음·저장 상태와 AI 분석 결과는 유지됩니다.', '초기화');
    } catch (reason) {
      error = `${archiveError(reason)} 확인 창을 열지 못해 초기화하지 않았습니다.`;
      return;
    }
    if (!confirmed || resetPending) return;
    resetPending = true;
    try {
      if (await action('personalization_reset')) {
        drafts = { positive: '', negative: '' };
        if (editingId !== null) closeEditor(editingId);
        if (data) setProfile(data.profile);
      }
    } finally {
      resetPending = false;
    }
  }

  function edit(interest: PersonalInterest) {
    interestDraftRevision += 1;
    editingId = interest.id;
    name = interest.name;
    aliases = interest.aliases.join(', ');
    priority = interest.priority;
    polarity = interest.polarity;
  }

  function closeEditor(id: number) {
    document.getElementById(`interest-${id}`)?.hidePopover();
  }

  function draftChanged() {
    interestDraftRevision += 1;
  }

  function submitDraft(event: KeyboardEvent, direction: Polarity) {
    if (event.key !== 'Enter' || event.isComposing) return;
    event.preventDefault();
    void addInterest(direction);
  }
</script>

{#snippet chips(list: PersonalInterest[], label: string)}
  {#each list as interest (interest.id)}
    <li>
      <button class="interest-chip" class:learned={interest.origin === 'learned'} popovertarget={`interest-${interest.id}`} aria-label={`${interest.name} ${interest.origin === 'learned' ? '학습된 관심사' : label} 수정`} disabled={resetPending} onclick={() => edit(interest)}>{interest.name}</button>
      <Popover id={`interest-${interest.id}`} label={`${interest.name} 수정`} onclose={() => { if (editingId === interest.id) editingId = null; }}>
        {#if editingId === interest.id}
          {#if interest.origin === 'learned'}<p class="popover-title">학습된 관심사 · 근거 {interest.evidence_count}건</p>{#if interest.evidence_summary}<p>{interest.evidence_summary}</p>{/if}{/if}
          <label class="popover-field">이름<input data-autofocus bind:value={name} maxlength="80" oninput={draftChanged} /></label>
          <label class="popover-field">별칭<input bind:value={aliases} maxlength="240" placeholder="한국어·영어 표현을 쉼표로 구분" oninput={draftChanged} /></label>
          {#if invalidAliases}<p role="alert" class="archive-error">별칭은 최대 12개, 각 80자 이내로 입력해 주세요.</p>{/if}
          <div class="popover-field">우선순위<div class="segmented" role="group" aria-label="우선순위">{#each [1, 2, 3, 4, 5] as value}<button class:selected={priority === value} aria-pressed={priority === value} onclick={() => { priority = value; draftChanged(); }}>{value}</button>{/each}</div></div>
          <div class="popover-field">방향<div class="segmented" role="group" aria-label="관심 방향">{#each [['positive', '관심'], ['negative', '관심 없음']] as [value, text]}<button class:selected={polarity === value} aria-pressed={polarity === value} onclick={() => { polarity = value as Polarity; draftChanged(); }}>{text}</button>{/each}</div></div>
          <div class="popover-actions">
            <button class="button small" disabled={busy || !name.trim() || invalidAliases} onclick={() => saveInterest(interest)}>{interest.origin === 'learned' ? '직접 입력으로 저장' : '저장'}</button>
            <button class="text-button danger" disabled={busy} aria-label={`${interest.name} 삭제`} onclick={() => void removeInterest(interest)}>삭제</button>
          </div>
        {/if}
      </Popover>
    </li>
  {/each}
{/snippet}

<div id="personalization" class="personalization" aria-busy={busy}>
  {#if error}<p role="alert" class="archive-error">{error}</p>{/if}
  <p role="status" class="archive-feedback">{resetPending ? '개인화 데이터를 초기화하는 중입니다.' : notice}</p>

  {#if !desktop}
    <p class="muted">프로필과 관심사 저장은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if data}
    <h3 class="settings-group-title">프로필</h3>
    <div class="settings-group">
      <div class="settings-row profile-row">
        <label class="sr-only" for="profile-university">대학교</label><input id="profile-university" bind:value={university} maxlength="100" placeholder="대학교" disabled={resetPending} />
        <label class="sr-only" for="profile-department">학과</label><input id="profile-department" bind:value={department} maxlength="100" placeholder="학과" disabled={resetPending} />
        <label class="sr-only" for="profile-level">학업 단계</label><input id="profile-level" bind:value={studyLevel} maxlength="60" placeholder="학업 단계 (예: 학부 3학년)" disabled={resetPending} />
        <button class="button secondary small" type="button" disabled={busy} aria-label="프로필 저장" onclick={saveProfile}>저장</button>
      </div>
    </div>

    <div class="settings-group-title"><h3>관심 분야</h3><Hint label="관심 분야">추천 순위에만 쓰이며 이 기기에만 저장됩니다. 주제를 눌러 별칭과 우선순위를 바꿀 수 있습니다. 학습된 관심사는 저장하거나 관심 표시한 서로 다른 공지가 쌓이면 생기고, 근거는 180일 뒤 만료됩니다. 삭제한 관심사는 과거 행동만으로 다시 생기지 않습니다.</Hint></div>
    <div class="settings-group">
      {#each [['positive', '관심'], ['negative', '관심 없음']] as [direction, label]}
        {@const items = explicit.filter((interest) => interest.polarity === direction)}
        <div class="settings-row chip-row">
          <span class="row-label">{label}</span>
          <ul class="chip-list" aria-label={label}>
            {@render chips(items, label)}
            <li><input class="chip-input" bind:value={drafts[direction as Polarity]} maxlength="80" placeholder="+ 추가" aria-label={`${label} 주제 추가`} disabled={resetPending} onkeydown={(event) => submitDraft(event, direction as Polarity)} /></li>
          </ul>
        </div>
      {/each}
      <div class="settings-row chip-row">
        <span class="row-label">학습됨</span>
        {#if learned.length}<ul class="chip-list" aria-label="학습된 관심사">{@render chips(learned, '학습된 관심사')}</ul>
        {:else}<span class="row-detail chip-empty">아직 없음</span>{/if}
      </div>
    </div>

    <div class="settings-group">
      <div class="settings-row"><span class="row-label">행동으로 배우기<Hint label="행동으로 배우기">관심 표시와 저장만 학습합니다. 읽기와 검색은 학습하지 않습니다. 끄면 새 행동을 학습에 쓰지 않습니다.</Hint></span><button class="switch" class:on={data.learning_enabled} role="switch" aria-checked={data.learning_enabled} aria-label="관심사 학습" disabled={busy} onclick={() => void action('personalization_set_learning', { enabled: !data!.learning_enabled })}><span></span></button></div>
      <div class="settings-row"><span class="row-label">개인화 초기화</span><button class="button secondary small danger" type="button" disabled={busy} aria-label="개인화 데이터 초기화" onclick={resetPersonalization}>초기화</button></div>
    </div>
  {:else}
    <p class="loading" role="status">개인화 정보를 불러오는 중입니다…</p>
  {/if}
</div>
