<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import contract from './analysis-contract.json' with { type: 'json' };
  import type { AiJob, AiResult, AiSnapshot } from './types';
  import { archiveDate, archiveError } from './archive-format';

  let { documentId, revisionId, onsettings, onchange }: { documentId: number; revisionId: number; onsettings: () => void; onchange: () => void } = $props();
  let results = $state<AiResult[]>([]);
  let data = $state<AiSnapshot | null>(null);
  let job = $state<AiJob | null>(null);
  let loading = $state(true);
  let submitting = $state(false);
  let error = $state('');
  let fetchError = $state('');
  let selected = $state<number | null>(null);
  let feedback = $state('');
  let disposed = false;
  let polling = false;
  let current = $derived(results.find((result) => result.id === selected) ?? results[0]);
  let processing = $derived(job?.status === 'pending' || job?.status === 'running');
  let activeHash = $derived(data?.models.find((model) => model.id === data?.preferences.active_model_id)?.sha256);
  let outdated = $derived(Boolean(current && ((activeHash && current.model_sha256 !== activeHash)
    || (data && current.model_identity.max_tokens !== undefined && current.model_identity.max_tokens !== data.preferences.max_tokens)
    || current.prompt_version !== contract.prompt_version
    || current.schema_version !== contract.schema_version
    || (data?.analysis_contract && (current.prompt_version !== data.analysis_contract.prompt_version
      || current.schema_version !== data.analysis_contract.schema_version
      || (data.analysis_contract.configuration_hash && current.configuration_hash !== data.analysis_contract.configuration_hash))))));
  let jobError = $derived(job?.status === 'failed' ? job.error ?? '분석하지 못했습니다. 다시 요청해 주세요.' : '');
  let progress = $derived(data?.progress?.job_id === job?.id ? data?.progress : null);
  const phaseLabel: Record<string, string> = { preparing: '공지와 분석 입력을 준비하고 있습니다', integrity: '모델 파일 무결성을 확인하고 있습니다', waiting: 'AI 엔진이 준비되기를 기다리고 있습니다', inference: '요약과 근거를 생성하고 있습니다', validating: '결과와 원문 근거를 확인하고 있습니다', persisting: '분석을 저장하고 있습니다', cancelling: '분석을 중단하고 메모리를 정리하고 있습니다' };

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => { if (processing && !submitting) void refresh(); }, 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  });
  async function refresh() {
    if (polling) return;
    polling = true;
    try {
      const [analyses, snapshot, jobs] = await Promise.all([
        invoke<AiResult[]>('ai_results', { documentId, revisionId }), invoke<AiSnapshot>('ai_snapshot'),
        invoke<AiJob[]>('ai_document_jobs', { documentId, revisionId }),
      ]);
      if (disposed) return;
      const previous = job;
      // Revision-local terminal history is independent of the global history window.
      const nextJob = jobs.find((value) => value.status === 'running')
        ?? jobs.find((value) => value.status === 'pending') ?? jobs[0] ?? null;
      // The two reads can straddle the completion transaction. Observe its result
      // before stopping polling on a terminal status.
      const coherent = nextJob?.status === 'completed' && !analyses.some((value) => value.job_id === nextJob.id)
        ? await invoke<AiResult[]>('ai_results', { documentId, revisionId }) : analyses;
      if (disposed) return;
      results = coherent;
      data = snapshot;
      job = nextJob;
      fetchError = '';
      if (previous && ['pending', 'running'].includes(previous.status) && job?.status === 'completed') {
        selected = null; feedback = '분석을 저장했습니다. 다음에는 추론 없이 바로 표시합니다.'; onchange();
      }
    } catch (reason) { if (!disposed) fetchError = archiveError(reason); }
    finally { if (!disposed) loading = false; polling = false; }
  }
  async function analyze(regenerate: boolean) {
    if (submitting || processing) return;
    submitting = true; error = ''; feedback = '';
    try {
      const response = await invoke<{ cached: AiResult | null; job: AiJob | null }>('ai_analyze', { documentId, revisionId, regenerate });
      if (disposed) return;
      job = response.job;
      if (response.cached) { results = [response.cached, ...results.filter((value) => value.id !== response.cached!.id)]; selected = response.cached.id; feedback = '저장된 분석을 사용했습니다. 새 추론은 실행하지 않았습니다.'; }
      else feedback = '분석을 요청했습니다. 원문은 계속 읽을 수 있습니다.';
    } catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { if (!disposed) submitting = false; }
  }
  async function cancel() {
    if (!job || submitting) return;
    submitting = true;
    try { await invoke('ai_cancel', { id: job.id }); if (!disposed) { await refresh(); feedback = '분석을 취소했습니다. 기존 결과는 유지됩니다.'; } }
    catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { if (!disposed) submitting = false; }
  }
</script>

<section class="ai-analysis" aria-label="공지 AI 분석" aria-busy={loading || submitting}>
  <div class="ai-model-heading"><h3>AI로 핵심 살펴보기</h3><span class="ai-tag">기기 내 분석</span></div>
  <p class="footnote">AI가 정리한 참고 정보입니다. 신청 조건과 일정은 아래 원문에서 반드시 확인하세요.</p>
  {#if loading}<p role="status">저장된 분석을 확인하는 중입니다…</p>{/if}
  {#if error || jobError}<p class="archive-error" role="alert">{error || jobError}</p>{/if}
  {#if fetchError}<p class="archive-error" role="alert">{fetchError}</p><button class="text-button" onclick={refresh}>분석 정보 다시 확인</button>{/if}
  <p class="ai-feedback" role="status">{feedback}</p>
  {#if processing}<div class="ai-operation"><p role="status">{progress ? (phaseLabel[progress.phase] ?? '로컬 AI 작업을 처리하고 있습니다') : job?.status === 'running' ? '이 기기에서 공지를 분석하고 있습니다…' : '다른 분석이 끝나기를 기다리고 있습니다…'}{#if progress}{' · 경과 '}{(progress.elapsed_ms / 1000).toFixed(0)}초{/if}</p><button class="text-button" disabled={submitting} onclick={cancel}>이 공지 분석 취소</button></div>{/if}
  {#if current}
    <div class="ai-analysis-meta"><span class="ai-tag">AI 분류 · {current.result.category}</span><span>저장된 분석 · {current.model_identity.name ?? '로컬 모델'}</span>{#if current.result.uncertainty !== 'clear'}<span>해석이 불확실할 수 있음</span>{/if}</div>
    {#if outdated}<p class="ai-caution">이 결과는 현재 모델 또는 분석 방식·설정과 다릅니다. 기존 결과를 그대로 읽거나 다시 생성할 수 있습니다.</p>{/if}
    {#if current.partial}<p class="ai-caution" role="note">긴 공지의 일부만 분석했습니다 ({current.processed_chars.toLocaleString('ko-KR')} / {current.total_chars.toLocaleString('ko-KR')}자). 일부 구간의 일정이나 조건이 빠졌을 수 있습니다. 원문 전체를 확인하세요.</p>{/if}
    <p class="ai-short-summary">{current.result.short_summary}</p><p class="ai-summary">{current.result.summary}</p>
    {#if current.result.facts.length}<h4>일정과 해야 할 일</h4><ul class="ai-facts">{#each current.result.facts as fact}<li><strong>{fact.label}</strong><p>{fact.value}{#if fact.date} <time datetime={fact.date}>· {fact.date} (원문에서 추출)</time>{/if}{#if fact.uncertainty !== 'clear'} <span class="muted">· {fact.uncertainty === 'unknown' ? '확인 필요' : '해석이 모호함'}</span>{/if}</p><details><summary>원문 근거</summary><blockquote>{fact.evidence}</blockquote></details></li>{/each}</ul>{/if}
    <details class="ai-provenance"><summary>분석 정보와 이전 결과</summary><p>{archiveDate(current.created_at, true)} · {(current.duration_ms / 1000).toFixed(1)}초 · 공지 버전 {current.revision_id} · 프롬프트 {current.prompt_version} / 형식 {current.schema_version}</p><code>모델 SHA-256 {current.model_sha256}</code>{#if results.length > 1}<label class="field-label" for={`analysis-history-${documentId}`}>저장된 분석 선택</label><select id={`analysis-history-${documentId}`} value={current.id} onchange={(event) => selected = Number(event.currentTarget.value)}>{#each results as result}<option value={result.id}>{archiveDate(result.created_at, true)} · {result.model_identity.name ?? '로컬 모델'} · #{result.id}</option>{/each}</select>{/if}</details>
  {:else if !loading && !processing && !fetchError}<p class="muted">아직 분석하지 않은 공지입니다. 원문을 외부로 보내지 않고 한국어 요약, 분류와 일정의 근거를 정리합니다.</p>{/if}
  {#if !loading && !processing && !fetchError}
    <div class="ai-actions">{#if data?.preferences.active_model_id}<button class="button secondary small" disabled={submitting || Boolean(data.operation)} onclick={() => analyze(Boolean(current))}>{current ? '이 공지 분석 다시 생성' : job?.status === 'failed' ? '이 공지 분석 다시 시도' : '이 공지 분석하기'}</button>{:else}<p class="muted">분석하려면 로컬 모델을 설치하고 선택해 주세요. 기존 분석과 원문은 모델 없이도 읽을 수 있습니다.</p>{/if}<button class="text-button" onclick={onsettings}>AI 모델 설정</button></div>
  {/if}
</section>
