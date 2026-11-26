#!/usr/bin/perl
# The first dump of ExifTool's tables (docs/plans/EXIFTOOL_RUST_REWRITE_DESIGN.md, "ExifTool's knowledge", step 1).
#
# usage: perl dump.pl EXIFTOOL_LIB OUT_DIR
#
# Loads ExifTool with no config file, builds every table the way ExifTool's own BuildTagLookup does
# (LoadAllTables: every module's tables, the Extra and Composite tables, every table a SubDirectory names, and the
# tags write procs add), then writes JSON Lines, one record per line:
#
#   <Module>.jsonl   {"kind":"table"}   a table: its name and its special (upper-case) entries
#                    {"kind":"tag"}     one tag definition: table, ID, index in the ID's list (null if alone), fields
#                    {"kind":"shared"}  a hash or array referenced from more than one place, or a named package
#                                       variable; written once, before its first use, and referred to as {"ref":ID}
#   code.jsonl       {"kind":"code"}    a code reference: a named sub's full name, or an anonymous sub's file, line
#                                       and deparsed source
#   lang/<xx>.jsonl  {"kind":"translate"}  a language module's translations
#   shortcuts.jsonl  {"kind":"shortcut"}   Shortcuts::Main
#   meta.json        versions and counts
#
# Values: a byte string is a JSON string (its bytes read as UTF-8); a string with Perl's UTF-8 flag is {"u":...};
# bytes that are not UTF-8 are {"bytes":HEX}; a number stored as a number (not as text) is a JSON number, or
# {"num":"%.17g"} where Perl's text form would lose precision; {"table":NAME} a reference to a table;
# {"tagref":"TABLE|ID|INDEX"} a reference to a tag definition; {"code":ID}; {"qr":TEXT}; {"sref":VALUE}.
#
# Strings (conditions, conversions) are written as their Perl source. ExifTool evaluates them in package
# Image::ExifTool (GetTagInfo, FoundTag, GetValue in ExifTool.pm, Writer.pl), whatever the table's module; meta.json
# records that.
use strict;
use warnings;
use B ();
use B::Deparse ();
use JSON::PP ();
use Scalar::Util qw(refaddr reftype blessed looks_like_number);
use File::Path qw(make_path);

my ($lib, $out) = @ARGV;
die "usage: dump.pl EXIFTOOL_LIB OUT_DIR\n" unless defined $out;
$lib =~ s{/+$}{};

# no config file: set before ExifTool loads, as its documentation says
{ no warnings 'once'; $Image::ExifTool::configFile = ''; }
unshift @INC, $lib;
require Image::ExifTool;

my @loadWarnings;
{
    local $SIG{__WARN__} = sub { push @loadWarnings, $_[0] };
    Image::ExifTool::LoadAllTables();
    require Image::ExifTool::Shortcuts;
}
no warnings 'once';
my %allTables = %Image::ExifTool::allTables;
for my $special ('Image::ExifTool::Extra', 'Image::ExifTool::Composite') {
    $allTables{$special} ||= Image::ExifTool::GetTagTable($special);
}
my @tableNames = sort keys %allTables;
# the flattened tags of XMP structures, as BuildTagLookup adds them (BuildTagLookup.pm 877-880): ExifTool makes
# them at run time, from the structures' definitions
{
    local $SIG{__WARN__} = sub { push @loadWarnings, $_[0] };
    for my $name (@tableNames) {
        my $table = $allTables{$name};
        my $vars = $table->{VARS} || {};
        next unless ($table->{GROUPS} and ($table->{GROUPS}{0} // '') eq 'XMP') or $vars->{ADD_FLATTENED};
        Image::ExifTool::XMP::AddFlattenedTags($table);
    }
}

# --- what is what, by address ---------------------------------------------------------------------------------------
my (%tableOf, %tagLoc, %varName);
$tableOf{refaddr $allTables{$_}} = $_ for @tableNames;

sub NumbersFirst {
    my ($x, $y) = @_;
    my ($nx, $ny) = (looks_like_number($x), looks_like_number($y));
    return $nx && $ny ? ($x <=> $y || $x cmp $y) : $nx ? -1 : $ny ? 1 : $x cmp $y;
}

# every table's definitions, in a fixed order
my %defs;    # table name => [ [id, index or undef, tagInfo], ... ]
for my $name (@tableNames) {
    my $table = $allTables{$name};
    my @list;
    for my $id (sort { NumbersFirst($a, $b) } Image::ExifTool::TagTableKeys($table)) {
        my @infos = Image::ExifTool::GetTagInfoList($table, $id);
        for my $i (0 .. $#infos) {
            push @list, [$id, @infos > 1 ? $i : undef, $infos[$i]];
            $tagLoc{refaddr $infos[$i]} //= join '|', $name, $id, $i;
        }
    }
    $defs{$name} = \@list;
}
my %special = map { $_ => 1 } keys %Image::ExifTool::specialTags;

# names of package variables, so a shared table keeps the name ExifTool gives it
sub WalkStash {
    my ($pkg, $seen) = @_;
    no strict 'refs';
    my $stash = \%{"${pkg}::"};
    for my $sym (sort keys %$stash) {
        if ($sym =~ /^(.+)::$/) {
            my $sub = "${pkg}::$1";
            WalkStash($sub, $seen) unless $seen->{$sub}++;
            next;
        }
        my $glob = $stash->{$sym};
        next unless ref \$glob eq 'GLOB';
        if (my $h = *{$glob}{HASH}) { $varName{refaddr $h} //= "%${pkg}::$sym" }
        if (my $a = *{$glob}{ARRAY}) { $varName{refaddr $a} //= "\@${pkg}::$sym" }
    }
}
WalkStash('Image::ExifTool', {});

# --- pass 1: how often each hash and array is referenced -----------------------------------------------------------
my %count;
sub Count {
    my ($v) = @_;
    return unless ref $v;
    my $addr = refaddr $v;
    return if $tableOf{$addr} or $tagLoc{$addr} or reftype($v) eq 'CODE' or re::is_regexp($v);
    return if $count{$addr}++;
    my $type = reftype $v;
    if ($type eq 'HASH') { Count($v->{$_}) for keys %$v }
    elsif ($type eq 'ARRAY') { Count($_) for @$v }
    elsif ($type eq 'SCALAR' or $type eq 'REF') { Count($$v) }
}
sub TagFields {
    my ($info) = @_;
    return grep { $_ ne 'Table' and $_ ne 'TagID' } sort keys %$info;
}
for my $name (@tableNames) {
    my $table = $allTables{$name};
    for my $key (grep { $special{$_} } keys %$table) { Count($table->{$key}) }
    for my $def (@{ $defs{$name} }) {
        my $info = $def->[2];
        Count($info->{$_}) for TagFields($info);
    }
}

# --- pass 2: encode -------------------------------------------------------------------------------------------------
my $json = JSON::PP->new->canonical(1)->utf8(1);
my (%sharedId, %codeId, @codeRecords, %stats, %inPath);
my $pending;    # records to write before the current one (shared values first used inside it)
my $nextShared = 0;

sub EncodeScalar {
    my ($slot) = @_;
    my $v = $$slot;
    my $flags = B::svref_2object($slot)->FLAGS;
    if (!($flags & B::SVf_POK) and ($flags & (B::SVf_IOK | B::SVf_NOK))) {
        my $text = "$v";
        return $v + 0 if $text + 0 == $v;
        ++$stats{'numbers needing %.17g'};
        return { num => sprintf('%.17g', $v) };
    }
    if (utf8::is_utf8($v)) {
        ++$stats{'strings with the UTF-8 flag'};
        return { u => $v };
    }
    return $v unless $v =~ /[\x80-\xff]/;
    my $copy = $v;
    if (utf8::decode($copy)) {
        ++$stats{'non-ASCII byte strings (UTF-8)'};
        return $copy;
    }
    ++$stats{'byte strings not UTF-8'};
    return { bytes => unpack('H*', $v) };
}

sub CodeId {
    my ($code) = @_;
    my $addr = refaddr $code;
    return $codeId{$addr} if $codeId{$addr};
    my $cv = B::svref_2object($code);
    my $gv = $cv->GV;
    my $file = $cv->FILE;
    $file =~ s{^\Q$lib\E/}{};
    my $line;
    for (my $op = $cv->START; $op and $$op; $op = $op->next) {
        if ($op->isa('B::COP')) { $line = $op->line; last }
    }
    my $named = $gv && $$gv && $gv->NAME ne '__ANON__';
    my ($id, %rec);
    if ($named) {
        $id = '&' . $gv->STASH->NAME . '::' . $gv->NAME;
        %rec = (kind => 'code', id => $id, named => JSON::PP::true, file => $file, line => $line);
        ++$stats{'code: named subs'};
    } else {
        $id = "anon:$file:" . ($line // '?');
        $id .= '#' . (1 + grep { $_->{id} =~ /^\Q$id\E(#|$)/ } @codeRecords);
        my $text = eval { B::Deparse->new->coderef2text($code) };
        %rec = (kind => 'code', id => $id, named => JSON::PP::false, file => $file, line => $line,
                deparsed => $text // undef, deparse_error => $text ? undef : "$@");
        ++$stats{$text ? 'code: anonymous subs' : 'code: anonymous subs not deparsed'};
    }
    push @codeRecords, \%rec;
    return $codeId{$addr} = $id;
}

sub Encode {
    my ($slot) = @_;
    my $v = $$slot;
    return undef unless defined $v;
    return EncodeScalar($slot) unless ref $v;
    my $addr = refaddr $v;
    return { table => $tableOf{$addr} } if $tableOf{$addr};
    return { tagref => $tagLoc{$addr} } if $tagLoc{$addr};
    return { code => CodeId($v) } if reftype($v) eq 'CODE';
    return { qr => "$v" } if re::is_regexp($v);
    if (($count{$addr} // 0) > 1 or $varName{$addr}) {
        return { ref => $sharedId{$addr} } if $sharedId{$addr};
        my $id = $sharedId{$addr} = $varName{$addr} // sprintf('shared%05d', ++$nextShared);
        my $value = EncodeInner($v);
        push @$pending, { kind => 'shared', id => $id, value => $value };
        ++$stats{'shared values'};
        return { ref => $id };
    }
    return EncodeInner($v);
}

sub EncodeInner {
    my ($v) = @_;
    my $addr = refaddr $v;
    if ($inPath{$addr}) {
        ++$stats{'cycles'};
        return { cycle => $sharedId{$addr} // "$addr" };
    }
    local $inPath{$addr} = 1;
    my $type = reftype $v;
    my $enc;
    # (sorted, so shared values are numbered and written in the same order on every run)
    if ($type eq 'HASH') { $enc = { map { $_ => Encode(\$v->{$_}) } sort keys %$v } }
    elsif ($type eq 'ARRAY') { $enc = [ map { Encode(\$v->[$_]) } 0 .. $#$v ] }
    elsif ($type eq 'SCALAR' or $type eq 'REF') { $enc = { sref => Encode($v) } }
    elsif ($type eq 'GLOB') { $enc = { glob => "" . *$v } }
    else { $enc = { other => $type } }
    my $class = blessed $v;
    $enc = { blessed => $class, value => $enc } if defined $class;
    return $enc;
}

# --- write ----------------------------------------------------------------------------------------------------------
make_path("$out/lang");
my %files;
sub Emit {
    my ($file, $record) = @_;
    my $fh = $files{$file} //= do {
        open my $h, '>:raw', "$out/$file" or die "$out/$file: $!\n";
        $h;
    };
    print $fh $json->encode($_), "\n" for @$pending;
    @$pending = ();
    print $fh $json->encode($record), "\n";
}

my %convFields = map { $_ => 1 } qw(Condition RawConv ValueConv PrintConv RawConvInv ValueConvInv PrintConvInv Hook);
for my $name (@tableNames) {
    my $table = $allTables{$name};
    (my $module = $name) =~ s/^Image::ExifTool:://;
    $module = $module =~ /^([^:]+)::/ ? $1 : 'ExifTool';
    my $file = "$module.jsonl";
    $pending = [];
    my %spec = map { $_ => Encode(\$table->{$_}) } grep { $special{$_} } sort keys %$table;
    Emit($file, { kind => 'table', name => $name, module => $module, special => \%spec });
    ++$stats{tables};
    for my $def (@{ $defs{$name} }) {
        my ($id, $index, $info) = @$def;
        my %fields = map { $_ => Encode(\$info->{$_}) } TagFields($info);
        Emit($file, { kind => 'tag', table => $name, id => "$id", index => $index, fields => \%fields });
        ++$stats{'tag definitions'};
        ++$stats{'tag IDs with a list of definitions'} if defined $index and $index == 0;
        for my $f (grep { $convFields{$_} } keys %$info) {
            my $v = $info->{$f};
            my $kind = !defined $v ? 'undef' : !ref $v ? 'string' : reftype($v) eq 'CODE' ? 'code' : reftype($v) eq 'HASH' ? 'hash'
                     : reftype($v) eq 'ARRAY' ? 'array' : lc reftype($v);
            ++$stats{"field $f: $kind"};
        }
    }
}

# languages
opendir my $dh, "$lib/Image/ExifTool/Lang" or die "$lib/Image/ExifTool/Lang: $!\n";
my @langs = sort map { /^(\w+)\.pm$/ ? $1 : () } readdir $dh;
closedir $dh;
for my $lang (@langs) {
    require "Image/ExifTool/Lang/$lang.pm";
    no strict 'refs';
    my $translate = \%{"Image::ExifTool::Lang::${lang}::Translate"};
    $pending = [];
    for my $tag (sort keys %$translate) {
        Emit("lang/$lang.jsonl", { kind => 'translate', lang => $lang, tag => $tag, value => Encode(\$translate->{$tag}) });
        ++$stats{'translations'};
    }
}

# shortcuts
$pending = [];
for my $name (sort keys %Image::ExifTool::Shortcuts::Main) {
    Emit('shortcuts.jsonl', { kind => 'shortcut', name => $name, value => Encode(\$Image::ExifTool::Shortcuts::Main{$name}) });
}

# code (after everything, so every reference has been seen)
$pending = [];
Emit('code.jsonl', $_) for sort { $a->{id} cmp $b->{id} } @codeRecords;
close $_ for values %files;

open my $meta, '>:raw', "$out/meta.json" or die "$out/meta.json: $!\n";
print $meta JSON::PP->new->canonical(1)->pretty->utf8->encode({
    exiftool_version => "$Image::ExifTool::VERSION",
    perl_version     => sprintf('%vd', $^V),
    deparse_version  => "$B::Deparse::VERSION",
    eval_package     => 'Image::ExifTool',
    config_file      => 'none',
    languages        => \@langs,
    load_warnings    => \@loadWarnings,
    counts           => \%stats,
});
close $meta;
printf STDERR "%-45s %s\n", $_, $stats{$_} for sort keys %stats;
print STDERR "load warnings: ", scalar(@loadWarnings), "\n";
