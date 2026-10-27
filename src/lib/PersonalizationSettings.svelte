<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { PersonalInterest, PersonalInterestInput, PersonalizationSnapshot } from './types';
  import { archiveError } from './archive-format';

  let { desktop }: { desktop: boolean } = $props();
  let data = $state<PersonalizationSnapshot | null>(null);
  let busy = $state(false);
  let resetPending = $state(false);
  let error = $state('');
  let notice = $state('');
  let university = $state('');
  let department = $state('');
  let studyLevel = $state('');
  let name = $state('');
  let aliases = $state('');
  let priority = $state(3);
  let polarity = $state<'positive' | 'negative'>('positive');
  let editingId = $state<number | null>(null);
  let interestDraftRevision = $state(0);
  let parsedAliases = $derived(aliases.split(/[,，;；\n]/).map((item) => item.trim()).filter(Boolean));
  let invalidAliases = $derived(parsedAliases.length > 12 || parsedAliases.some((alias) => Array.from(alias).length > 80));

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

  async function saveInterest() {
    const value = name.trim();
    if (!value) return;
    if (invalidAliases) {
      error = '별칭은 최대 12개까지 입력하고, 각 별칭은 80자 이내여야 합니다.';
      return;
    }
    const draftRevision = interestDraftRevision;
    const saved = await action('personalization_save_interest', {
      interest: {
        id: editingId,
        name: value,
        aliases: parsedAliases,
        priority,
        polarity,
      } satisfies PersonalInterestInput,
    });
    if (saved && interestDraftRevision === draftRevision) clearEditor();
  }

  async function removeInterest(id: number) {
    const draftRevision = interestDraftRevision;
    const removed = await action('personalization_remove_interest', { id });
    if (!removed || editingId !== id) return;
    if (interestDraftRevision === draftRevision) clearEditor();
    else {
      editingId = null;
      interestDraftRevision += 1;
    }
  }

  async function resetPersonalization() {
    if (!confirm('개인화 프로필, 관심사, 기억과 피드백을 모두 초기화할까요? 보관한 공지는 유지됩니다.')) return;
    resetPending = true;
    try {
      if (await action('personalization_reset')) {
        clearEditor();
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

  function clearEditor() {
    interestDraftRevision += 1;
    editingId = null;
    name = '';
    aliases = '';
    priority = 3;
    polarity = 'positive';
  }

  function onInterestDraftInput() {
    interestDraftRevision += 1;
  }
</script>

<section id="personalization" class="settings-section" aria-busy={busy}>
  <div class="section-heading">
    <h2>개인화</h2>
    <p>관심사와 학업 정보를 이 기기에 저장합니다. 추천은 설명 가능한 규칙으로 정렬됩니다.</p>
  </div>
  {#if error}<p role="alert" class="archive-error">{error}</p>{/if}
  <p role="status" class="archive-feedback">{resetPending ? '개인화 데이터를 초기화하는 중입니다.' : notice}</p>

  {#if !desktop}
    <p class="muted">프로필과 관심사 저장은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if data}
    <div class="form-grid">
      <label class="field-label">대학교<input bind:value={university} maxlength="100" disabled={resetPending} /></label>
      <label class="field-label">학과<input bind:value={department} maxlength="100" disabled={resetPending} /></label>
      <label class="field-label">학업 단계<input bind:value={studyLevel} maxlength="60" placeholder="예: 학부 3학년" disabled={resetPending} /></label>
    </div>
    <button class="button small" type="button" disabled={busy} onclick={saveProfile}>프로필 저장</button>

    <h3>관심 분야</h3>
    {#if data.interests.length}
      <ul aria-label="저장된 관심 분야">
        {#each data.interests as interest (interest.id)}
          <li class="setting-row">
            <div>
              <strong>{interest.name}</strong>
              <p>
                {interest.aliases.join(', ') || '별칭 없음'} · 우선순위 {interest.priority}
                · {#if interest.origin === 'explicit'}직접 입력 · {interest.polarity === 'negative' ? '덜 보고 싶은 주제' : '더 보고 싶은 주제'}{:else}행동 근거 기반 학습 제안 · 근거 {interest.evidence_count}건{/if}
              </p>
              {#if interest.evidence_summary}<small>{interest.evidence_summary}</small>{/if}
            </div>
            <button class="text-button" type="button" disabled={busy} onclick={() => edit(interest)}>
              {interest.origin === 'learned' ? '직접 입력으로 수정' : '수정'}
            </button>
            <button class="text-button" type="button" disabled={busy} onclick={() => void removeInterest(interest.id)}>삭제</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">아직 저장된 관심 분야가 없습니다. 자주 찾는 주제부터 추가해 보세요.</p>
    {/if}
    <p class="footnote">관심사를 삭제하면 과거 행동만으로 다시 나타나지 않습니다. 새로 저장하거나 관심 표시한 소식이 쌓이면 다시 제안될 수 있습니다.</p>

    <div class="personalization-editor">
      <label class="field-label">관심사 이름<input bind:value={name} maxlength="80" oninput={onInterestDraftInput} disabled={resetPending} /></label>
      <label class="field-label">별칭<input bind:value={aliases} maxlength="240" placeholder="한국어·영어 표현을 쉼표로 구분" oninput={onInterestDraftInput} disabled={resetPending} /></label>
      {#if invalidAliases}<small role="alert" class="archive-error">별칭은 최대 12개까지 입력하고, 각 별칭은 80자 이내여야 합니다.</small>{/if}
      <label class="field-label">우선순위/강도 (5가 가장 높음)
        <select bind:value={priority} onchange={onInterestDraftInput} disabled={resetPending}>
          {#each [1, 2, 3, 4, 5] as value}<option value={value}>{value}</option>{/each}
        </select>
      </label>
      <label class="field-label">관심 방향
        <select bind:value={polarity} onchange={onInterestDraftInput} disabled={resetPending}>
          <option value="positive">더 보고 싶은 주제</option>
          <option value="negative">덜 보고 싶은 주제</option>
        </select>
      </label>
      <div class="reader-actions">
        <button class="button small" type="button" disabled={busy || !name.trim() || invalidAliases} onclick={saveInterest}>
          {editingId === null ? '관심사 추가' : '관심사 변경 저장'}
        </button>
        {#if editingId !== null}<button class="text-button" type="button" disabled={busy} onclick={clearEditor}>수정 취소</button>{/if}
      </div>
    </div>

    <div class="setting-row">
      <div>
        <h3>행동으로 배우기</h3>
        <p>관심 표시와 저장만 새 기억에 반영합니다. 읽기는 학습하지 않습니다. 근거는 180일 뒤 만료됩니다. 끄면 새 행동은 학습에 쓰지 않습니다.</p>
      </div>
      <button
        class="switch"
        role="switch"
        aria-checked={data.learning_enabled}
        aria-label="관심사 학습"
        disabled={busy}
        onclick={() => void action('personalization_set_learning', { enabled: !data!.learning_enabled })}
      ><span></span></button>
    </div>
    <button class="text-button" type="button" disabled={busy} onclick={resetPersonalization}>개인화 데이터 초기화</button>
  {:else}
    <p>개인화 정보를 불러오는 중입니다…</p>
  {/if}
</section>
