<script lang="ts">
  let { desktop }: { desktop: boolean } = $props();
</script>
<section id="about" class="settings-section about-section">
  <div class="section-heading"><h2>정보</h2><p>Rhythm Windows 버전과 업데이트 안내입니다.</p></div>
  <div class="about-brand"><h2>Rhythm</h2><span>0.9.3</span></div>
  <p>Windows 11 x64 · 로컬 CPU AI · MIT 라이선스</p>
  <div class="setting-row"><div><h3>앱 업데이트</h3><p>새 Windows 설치 파일로 직접 업데이트합니다. 앱 안 자동 업데이트는 제공하지 않습니다.</p></div></div>
  <ol class="manual-update">
    <li>새 설치 파일의 출처와 제공된 SHA-256을 확인합니다.</li>
    <li>Rhythm을 트레이 메뉴에서 완전히 종료합니다.</li>
    <li>새 설치 파일을 실행해 같은 사용자 계정에 설치합니다.</li>
    <li>Rhythm을 다시 엽니다. 공지, 설정과 모델은 프로그램과 별도 폴더에 유지됩니다.</li>
  </ol>
  <p class="footnote">이 배포 파일에는 정식 코드 서명이 없어 Windows 보안 경고가 나타날 수 있습니다.</p>
  {#if !desktop}<p class="footnote">설치와 업데이트 확인은 Windows 앱에서 진행하세요.</p>{/if}
</section>
