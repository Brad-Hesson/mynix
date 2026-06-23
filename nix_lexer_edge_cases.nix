# nix_lexer_edge_cases.nix
# A deliberately valid Nix expression intended to exercise lexer/parser edges.
# Suggested parse check: nix-instantiate --parse ./nix_lexer_edge_cases.nix

/* Long comment with tokens that must be ignored:
   "string" '' indented? ${interpolation?} // ++ -> == != <= >= && || # comment
   Asterisks: * ** *** and slashes: / // but no nested block comments.
**/

/** Doc-comment-shaped block comment. Newer Nix lexers distinguish this internally. */

let
  # Identifier edges: hyphen and apostrophe are part of an ID token if there is no whitespace.
  ident = "ID";
  hyphen-name = "hyphenated identifier, not subtraction";
  prime' = "identifier ending in apostrophe";
  _underscore123 = 123;

  # Same glyphs, different tokenization: this is subtraction because of whitespace.
  minusExpression = 10 - 3;

  base = {
    inherited = "from base";
    nested = { leaf = "leaf"; };
  };

  inheritedSet = {
    inherit ident hyphen-name prime';
    inherit (base) inherited;
    inherit (base.nested) leaf;
  };

  mk = x: y: { inherit x y; };
  patternLeftAt = args @ { a, b ? 2, ... }: args.a + b;
  patternRightAt = { a, b ? 2, ... } @ args: args.a + b;

  dynamicAttrName = "generated-name";
  pathStem = "generated";

in rec {
  commentsAreIgnored = true;

  literals = {
    boolTrue = true;
    boolFalse = false;
    nothing = null;

    ints = [ 0 1 007 9223372036854775807 ];

    # These match the Nix lexer FLOAT rule: digits-dot, optional fractional digits,
    # or optional leading zero before dot, with optional exponent.
    floats = [ 1. 1.0 0.5 .5 1.e3 1.0e-3 .25E+2 ];

    # Unary minus is not part of the numeric literal token.
    negatives = [ (-1) (-1.0) (0 - 1) ];
  };

  strings = {
    plainDouble = "plain string";
    doubleWithEscapes = "quote=\" backslash=\\ newline=\n tab=\t carriage=\r arbitrary=\z dollar-curly=\${";
    doubleWithDollarRuns = "plain-dollar=$ double-dollar-curly=$${ ends-with-dollar=$";
    doubleMultiline = "first physical line
second physical line with # not a comment and /* not a comment */
third physical line";

    doubleInterpolation = "outer ${ident}, nested ${"inner ${hyphen-name}"}, braces ${let x = { a = "}"; }; in x.a}";

    indentedBasic = ''
      one
       two
        three
    '';

    indentedOpenWhitespace = ''    
      spaces after the opening delimiters plus the newline are ignored
      when there is no non-whitespace text on that first line
    '';

    indentedSameLine = ''same-line start
      next line still belongs to the string
    '';

    indentedEscapes = ''
      double quotes do not need escaping: "hello"
      backslashes are mostly literal: \ \n \t
      single apostrophe is literal: it's fine
      literal dollar: ''$
      literal dollar-curly: ''${notInterpolation}
      double-dollar-curly is literal without escaping: $$ {
      actual double-dollar-curly adjacent: $${
      literal two apostrophes: '''
      escaped newline token here -> ''\n <- before this text
      escaped tab token here -> ''\t <- before this text
      arbitrary escaped character: ''\q
      interpolation still works: ${ident}
      nested expression: ${let x = { y = "nested"; }; in x.y}
    '';

    # The next line intentionally starts with an actual tab character inside the indented string.
    indentedWithTabPrefix = ''
	this line starts with a real tab, which Nix does not strip as indentation
      this line starts with spaces
    '';
  };

  attrSets = {
    simple = 1;
    nested.path.leaf = "attr path leaf";
    a-b = "attribute name containing hyphen";
    apostrophe' = "attribute name containing apostrophe";
    "quoted attr with spaces" = "quoted attr value";
    "quoted-${ident}-attr" = "quoted attribute with interpolation";
    ${dynamicAttrName} = "dynamic attribute name";
    ${"string-literal-dynamic-name"} = "dynamic attribute from string literal";
  };

  selections = {
    simple = attrSets.simple;
    nested = attrSets.nested.path.leaf;
    quoted = attrSets."quoted attr with spaces";
    interpolatedSelection = attrSets.${dynamicAttrName};
    defaultedSelection = attrSets.missing.path or "fallback";
    hasSimple = attrSets ? simple;
    hasNested = attrSets ? nested.path.leaf;
  };

  lists = {
    empty = [ ];
    mixed = [ 1 "two" true null { three = 3; } [ 4 5 ] (mk "x" "y") ];
    concat = [ 1 2 ] ++ [ 3 4 ];
  };

  functions = {
    identityResult = (x: x) "arg";
    curriedResult = (x: y: x + y) 1 2;
    patternLeftAtResult = patternLeftAt { a = 40; extra = "ignored by ellipsis"; };
    patternRightAtResult = patternRightAt { a = 41; extra = "ignored by ellipsis"; };
    setPatternDefaultString = ({ a, b ? "B", ... }: a + b) { a = "A"; c = "ignored"; };
  };

  operators = {
    arithmetic = {
      add = 1 + 2;
      subtract = 5 - 3;
      multiply = 2 * 3;
      divide = 9 / 3;
    };

    comparisons = [
      (1 < 2)
      (2 <= 2)
      (3 > 2)
      (3 >= 3)
      ("a" == "a")
      ("a" != "b")
    ];

    boolean = {
      notOp = ! false;
      andOp = true && false;
      orOp = true || false;
      implicationOp = true -> false;
    };

    update = { a = 1; b = 2; } // { b = 3; c = 4; };
  };

  control = {
    conditional = if 1 + 1 == 2 then "yes" else "no";
    assertion = assert 2 * 3 == 6; "assertion passed";
    letExpression = let local = "local"; in local;
    withExpression = with { scoped = "from with"; }; scoped;
  };

  paths = {
    absolute = /etc/passwd;
    relative = ./relative-file.nix;
    a = ./foo;
    b = ./foo.txt;
    c = ./foo-bar;
    d = ./foo_bar;
    e = ./foo+bar;
    f = foo/bar;
    g = foo/bar/baz;
    h = foo.bar/baz-qux+1;
    # parentRelative = ../parent-dir/file+name_1.2;
    # home = ~/.config/nix/nix.conf;
    lookup = <nixpkgs/lib>;

    # Interpolated path: path lexing must enter expression mode at ${...} and resume path mode.
    interpolatedRelative = ./${pathStem}-${ident}.nix;
    # interpolatedHome = ~/${pathStem}/config.nix;
  };

  # URI literals are a deprecated/convenience token family, but still accepted by Nix.
  # uris = {
  #   http = http://example.org/foo.tar.bz2;
  #   httpsQuery = https://example.org/a-b_c.d+e?x=1&y=$z;
  #   mailto = mailto:user+tag@example.org;
  # };

  recursion = rec {
    first = "a";
    second = first + "b";
  };

  inherited = inheritedSet;

  # The following are intentionally kept in comments because they are useful negative lexer tests:
  # invalidSingleQuotedString = 'not a Nix string';
  # invalidNestedComment = /* outer /* inner */ outer still open? */ 1;
  # invalidTrailingSlashPath = ./foo/;
  # experimentalPipeOperators = 1 |> (x: x) <| 2;
}
