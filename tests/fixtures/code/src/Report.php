<?php

namespace App;

class Report
{
    public function render(): string
    {
        return "report";
    }
}

function build_report(): Report
{
    return new Report();
}
