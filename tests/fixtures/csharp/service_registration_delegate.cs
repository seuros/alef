using System;
using Test;

static void Register(TestService service)
{
    Delegate handler = new Action(() => { });
    service.add_handler("/", handler);
}
