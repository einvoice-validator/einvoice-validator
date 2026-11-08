<?xml version="1.0" encoding="UTF-8"?>
<!-- Synthetic: patterns, rules and a phase taken from other files with sch:include and
     sch:extends href. -->
<schema xmlns="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt2" defaultPhase="main">
  <ns prefix="a" uri="urn:example:a"/>
  <phase id="main">
    <active pattern="Included"/>
    <active pattern="Local"/>
  </phase>
  <phase id="other">
    <active pattern="Local"/>
  </phase>
  <include href="parts/pattern.xml"/>
  <pattern id="Local">
    <rule context="a:Order">
      <extends href="parts/rule.xml"/>
      <assert id="L-1" test="a:Date">An order has a date.</assert>
    </rule>
  </pattern>
  <pattern id="Unused">
    <rule context="a:Never">
      <report id="U-1" test="true()">Never reported in phase main.</report>
    </rule>
  </pattern>
</schema>
