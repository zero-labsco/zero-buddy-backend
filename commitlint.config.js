// Conventional Commits 校验规则（见组织 CONTRIBUTING.md）
// 允许的前缀：feat / fix / docs / chore / refactor / perf / test / build / ci / style / revert
module.exports = {
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
  },
};
