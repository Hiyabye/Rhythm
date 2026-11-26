-- Running out of an import's attachment budget used to mark the remaining files
-- too_large, and images on routes approved later (/dataview/board/, other
-- public HTTPS hosts; user decision, #78) were marked unsupported. Both become
-- pending so a download run can retry them; a file that really is too large or
-- unsupported gets its terminal state back on that attempt.
UPDATE resources SET status='pending', error=NULL WHERE status='too_large';
UPDATE resources SET status='pending', error=NULL
 WHERE status='unsupported' AND kind='image'
   AND error='검증된 학교 공개 자료 주소가 아니어서 자동 다운로드하지 않았습니다.';
PRAGMA user_version=12;
