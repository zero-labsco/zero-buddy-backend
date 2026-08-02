// Conventional Commits 校验规则（见组织 CONTRIBUTING.md）
// 允许的前缀：feat / fix / docs / chore / refactor / perf / test / build / ci / style / revert
export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'type-enum': [
      2,
      'always',
      [
        'feat',
        'fix',
        'docs',
        'chore',
        'refactor',
        'perf',
        'test',
        'build',
        'ci',
        'style',
        'revert',
      ],
    ],
    'subject-case': [0],
    'header-max-length': [2, 'always', 100],
    // Dependabot 自动升级 PR 的 body 含长 URL，放宽以避免误拒
    'body-max-line-length': [0],
    'footer-max-line-length': [0],
  },
};
