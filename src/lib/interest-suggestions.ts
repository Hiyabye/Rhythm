// Built-in, offline suggestions for the interest picker (no network, no AI). Top-level labels
// are broad categories for Sogang notices; `children` are narrower, related interests that
// appear once the parent is chosen. Names that the backend already canonicalizes
// (`storage/personalization/topics.rs`) use that canonical display name so a chosen chip is
// stored under the same key as a typed one. Labels must be unique across the whole tree and
// must not contain the sensitive-term markers listed there (tests/ui/interest-picker.spec.ts
// checks both).
export interface InterestSuggestion {
  label: string;
  children?: InterestSuggestion[];
}

const leaf = (...labels: string[]): InterestSuggestion[] => labels.map((label) => ({ label }));

export const interestSuggestions: InterestSuggestion[] = [
  { label: '장학금', children: leaf('교내 장학금', '국가장학금', '외부 장학금', '해외 장학금') },
  {
    label: '취업·채용',
    children: [
      { label: '채용설명회', children: leaf('IT·개발', '금융', '공공기관') },
      ...leaf('신입 공채', '직무 멘토링', '진로 특강'),
    ],
  },
  { label: '인턴십', children: leaf('체험형 인턴', '기업 인턴', '해외 인턴') },
  { label: '공모전', children: leaf('아이디어 공모전', '해커톤', '디자인 공모전', '논문 공모전') },
  {
    label: '연구',
    children: [
      ...leaf('학부연구생', '논문·학회'),
      { label: '인공지능', children: leaf('머신러닝', '데이터 분석', 'AI 인프라') },
    ],
  },
  { label: '교환학생', children: leaf('교환학생 파견', '방문학생', '어학연수', '글로벌 프로그램') },
  { label: '학사', children: leaf('수강신청', '졸업·학위', '휴학·복학', '복수전공') },
  { label: '행사', children: leaf('특강·세미나', '축제', '동아리·학회') },
  { label: '봉사활동', children: leaf('교내 봉사', '해외 봉사', '멘토링') },
  { label: '창업', children: leaf('창업 동아리', '창업 경진대회', '창업 지원금') },
];
